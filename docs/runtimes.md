# Runtime Modes

| Mode | Description | Best for |
|------|-------------|----------|
| **Auto** (default) | Detects Docker, then Podman, then falls back to emulation | Most users |
| **Docker** | Full container isolation, closest to GitHub runners | Production, CI/CD |
| **Podman** | Rootless containers, no daemon required | Security-conscious environments |
| **Kubernetes** | Each step runs as a pod in a cluster you point `kubectl` at | GPU/XPU jobs, shared build clusters, self-hosted forges |
| **Emulation** | Runs directly on host, no containers needed | Quick local testing |
| **Secure Emulation** | Sandboxed host processes with filesystem/network restrictions | Running untrusted workflows without a container runtime |

Only **Auto** falls back. If you pass `--runtime docker` or `--runtime podman` and it doesn't answer, the run fails rather than running your steps on this machine.

## Kubernetes runtime

`--runtime kubernetes` dispatches every step to the cluster `kubectl` is configured for. It is never auto-detected, and it never falls back to running on your machine: if the cluster is unreachable, the run fails.

```bash
# Pick the cluster and namespace, then run as usual
wrkflw run --runtime kubernetes --k8s-context gpu-cluster --k8s-namespace ci .github/workflows/train.yml

# Land on GPU nodes and request an accelerator for every step
wrkflw run --runtime kubernetes \
  --k8s-node-selector accelerator=a100 \
  --k8s-resource nvidia.com/gpu=1 --k8s-resource memory=16Gi \
  .github/workflows/train.yml

# Private registry, and a registry wrkflw can push locally built images to
wrkflw run --runtime kubernetes --k8s-image-pull-secret regcred \
  --k8s-registry registry.example.com/ci .github/workflows/ci.yml

# Every flag has a WRKFLW_K8S_* environment variable, so set them once
export WRKFLW_K8S_NAMESPACE=ci WRKFLW_K8S_RESOURCE=nvidia.com/gpu=1
```

| Flag | Environment variable | Purpose |
|------|----------------------|---------|
| `--k8s-context NAME` | `WRKFLW_K8S_CONTEXT` | kubeconfig context (default: current) |
| `--k8s-namespace NAME` | `WRKFLW_K8S_NAMESPACE` | Namespace pods are created in (default: current) |
| `--k8s-node-selector KEY=VALUE` | `WRKFLW_K8S_NODE_SELECTOR` | `nodeSelector` for every pod; repeatable |
| `--k8s-resource NAME=QUANTITY` | `WRKFLW_K8S_RESOURCE` | Limits and requests for the step container, e.g. `nvidia.com/gpu=1`; repeatable |
| `--k8s-image-pull-secret NAME` | `WRKFLW_K8S_IMAGE_PULL_SECRET` | `imagePullSecrets` for every pod; repeatable |
| `--k8s-registry REGISTRY` | `WRKFLW_K8S_REGISTRY` | Where images built locally are pushed so the cluster can pull them |
| `--k8s-helper-image IMAGE` | `WRKFLW_K8S_HELPER_IMAGE` | Image for the sync helpers (default `busybox:1.36`, pinned by digest; needs `sh` and `tar`) |
| `--k8s-timeout SECONDS` | `WRKFLW_K8S_TIMEOUT` | Per-step limit, pod creation to sync-back (default 21600, the engine's job timeout) |
| `--k8s-schedule-timeout SECONDS` | `WRKFLW_K8S_SCHEDULE_TIMEOUT` | How long a pod may stay unschedulable before the step fails (default 600) |

How a step runs:

1. A pod is created with one `emptyDir` per mount, an init container that waits for a ready marker, the step container, and a small sidecar. Pods are labelled `app.kubernetes.io/managed-by=wrkflw` and `wrkflw.dev/session=<id>`.
2. The workspace, GitHub env files and action checkouts are streamed into the init container as tar archives, then the marker is set and the step starts.
3. After the step exits, its logs are collected and every mount is streamed back out through the sidecar, so `GITHUB_OUTPUT`, artifacts and workspace changes reach the next step.
4. The pod is deleted. With `--preserve-containers-on-failure` a failed pod is kept for `kubectl logs <pod> -c main` and `kubectl exec -it <pod> -c wrkflw-sync -- sh`.

Every pod carries `activeDeadlineSeconds`, set to `--k8s-timeout` (at least 1) plus 30 seconds, so the cluster stops it even if wrkflw is killed. A preserved pod is stopped at that deadline too, and keeps its resources until then, GPUs included. Its logs stay readable until you delete it, but `kubectl exec` no longer works. `--preserve-containers-on-failure` also keeps a pod whose step timed out. After `--k8s-timeout` the deadline is only 30 seconds away, so read its logs instead. After a shorter `timeout-minutes`, `kubectl exec` works until the deadline. Ctrl+C always deletes pods.

Things to know:

- Only `kubectl` is needed on your machine. Jobs that build an image (`actions/setup-*` on a standard runner image, Dockerfile actions) also need Docker or Podman locally and `--k8s-registry`, because the cluster cannot see your local image store.
- Each step syncs the workspace and `RUNNER_TEMP` in and out, so large workspaces and large temporary files cost time per step.
- A failed image pull is retried for 60 seconds before the step fails, so a bad image name takes that long to report.
- Mounts are copies. Job `container.volumes` directories are copied in and written back after each step, so parallel jobs sharing one do not see each other's changes and the last to finish wins. Write-back only adds and updates: a file a step deletes from such a directory stays on the host, and files other processes create there are left alone. Only the job's workspace and `GITHUB_*` files mirror the pod exactly. A single-path volume such as `/cache` lives only in the pod and is never written back. Single files and sockets cannot be mounted and are skipped with a warning, sockets inside a mounted directory are not synced, and named volumes start empty.
- Logs are read from the kubelet when the step ends, so a step that outgrows the node's container log rotation (10 MiB by default) loses its earliest lines.
- A failed status check is retried for 60 seconds, so a brief API server outage does not end a running step. A deleted pod fails the step at once.
- A pod the scheduler cannot place fails the step after `--k8s-schedule-timeout`. Raise it if your autoscaler takes longer than 10 minutes to bring up a node.
- Synced files are owned by root in the pod, so run steps in images that run as root, as GitHub requires for container actions. Other users can read the workspace but not write to it, and namespaces enforcing the `restricted` Pod Security Standard are not supported.
- Stdout and stderr come back as one stream, as Kubernetes logs them.
- Helper containers declare small fixed CPU and memory requests and limits, so namespaces with a `ResourceQuota` accept them.
- Kubernetes validates environment variable names itself, and older clusters reject names Docker accepts. Such a step fails when its pod is created.
- Secrets reach the step as plain pod environment variables, visible to anyone who can read pods in that namespace. Pods do not mount a service account token.
- `--platform` and `runs-on` work as elsewhere: they pick the image. Node placement comes from `--k8s-node-selector`.
