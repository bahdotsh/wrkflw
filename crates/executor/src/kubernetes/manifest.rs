//! The pod a step runs in: which mounts it gets and how its manifest reads.
//! Everything here is pure, so it is tested without a cluster.

use super::{
    KubernetesConfig, DEADLINE_MARGIN_SECS, MAIN_CONTAINER, MANAGED_BY_LABEL, READY_MARKER,
    SESSION_ID, SESSION_LABEL, SYNC_CONTAINER, SYNC_DIR, SYNC_VOLUME, WAIT_CONTAINER,
};
use crate::environment::{CONTAINER_GITHUB_DIR, CONTAINER_WORKSPACE};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Container paths backed by directories the executor created for this job.
/// Nothing else on those host directories can be lost, so only these are
/// mirrored. `/github/action` is left out: a local action lives in the user's
/// repository. Exact paths, so a user volume nested below one does not qualify.
const MIRRORED_MOUNTS: &[&str] = &[CONTAINER_WORKSPACE, CONTAINER_GITHUB_DIR];

/// One host directory and the container path it is exposed at. An empty
/// `host` is a volume that exists only in the pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mount {
    pub(super) host: PathBuf,
    pub(super) container: PathBuf,
}

impl Mount {
    /// True when the host directory must end up exactly as the pod left it,
    /// deletions included. Any other mount belongs to the user and may hold
    /// files the pod never saw, so copy-out only adds to it.
    pub(super) fn mirrored(&self) -> bool {
        MIRRORED_MOUNTS
            .iter()
            .any(|path| self.container == Path::new(path))
    }
}

/// Turn the caller's `(host, container)` pairs into an ordered mount plan.
///
/// Mounts are sorted by container path so a parent mount is populated before
/// a mount nested inside it, and a container path requested twice keeps its
/// first host directory, as a `docker run` with duplicate `-v` flags would.
pub(super) fn plan_mounts(volumes: &[(&Path, &Path)]) -> Vec<Mount> {
    let mut mounts: Vec<Mount> = Vec::new();
    for (host, container) in volumes {
        // An emptyDir cannot stand in for a file or a host socket such as
        // /var/run/docker.sock, and a bare name is a Docker named volume.
        if host.exists() && !host.is_dir() {
            wrkflw_logging::warning(&format!(
                "Kubernetes: skipping mount of {}: only directories can be synced into a pod",
                host.display()
            ));
            continue;
        }
        if !host.is_absolute() && !host.as_os_str().is_empty() {
            wrkflw_logging::warning(&format!(
                "Kubernetes: mounting {} empty: host path '{}' is not absolute, \
                 and named volumes do not exist on a cluster",
                container.display(),
                host.display()
            ));
        }
        if mounts.iter().any(|m| m.container == *container) {
            wrkflw_logging::warning(&format!(
                "Kubernetes: container path {} is mounted twice; keeping the first mapping",
                container.display()
            ));
            continue;
        }
        mounts.push(Mount {
            host: host.to_path_buf(),
            container: container.to_path_buf(),
        });
    }
    mounts.sort_by(|a, b| a.container.cmp(&b.container));
    mounts
}

/// Paths nested inside `mount` that belong to other mounts, relative to
/// `mount.container` and prefixed with `./` the way `tar -C dir .` names
/// them. They are excluded when the parent is streamed back out so a nested
/// emptyDir is not copied into the parent's host directory.
pub(super) fn nested_excludes(mount: &Mount, all: &[Mount]) -> Vec<String> {
    all.iter()
        .filter(|other| other.container != mount.container)
        .filter_map(|other| other.container.strip_prefix(&mount.container).ok())
        .map(|rel| {
            let parts: Vec<_> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect();
            format!("./{}", parts.join("/"))
        })
        .collect()
}

/// True when `mount` sits inside another mount. The parent's archive writes
/// straight through into it, where Docker's bind mount would hide those files.
pub(super) fn is_nested(mount: &Mount, all: &[Mount]) -> bool {
    all.iter().any(|other| {
        other.container != mount.container && mount.container.starts_with(&other.container)
    })
}

/// Everything that goes into one pod, independent of the cluster.
pub(super) struct PodInput<'a> {
    pub(super) name: &'a str,
    pub(super) image: &'a str,
    pub(super) cmd: &'a [&'a str],
    pub(super) env_vars: &'a [(&'a str, &'a str)],
    pub(super) working_dir: &'a Path,
    pub(super) mounts: &'a [Mount],
    pub(super) entrypoint: Option<&'a str>,
}

/// Build the pod manifest. Pure, so it is unit-tested without a cluster.
pub(super) fn build_pod_manifest(config: &KubernetesConfig, input: &PodInput<'_>) -> Value {
    let volume_name = |i: usize| format!("vol-{}", i);

    let step_mounts: Vec<Value> = input
        .mounts
        .iter()
        .enumerate()
        .map(|(i, m)| json!({ "name": volume_name(i), "mountPath": m.container.to_string_lossy() }))
        .collect();
    let mut helper_mounts = step_mounts.clone();
    helper_mounts.push(json!({ "name": SYNC_VOLUME, "mountPath": SYNC_DIR }));

    let mut volumes: Vec<Value> = (0..input.mounts.len())
        .map(|i| json!({ "name": volume_name(i), "emptyDir": {} }))
        .collect();
    volumes.push(json!({ "name": SYNC_VOLUME, "emptyDir": {} }));

    let env: Vec<Value> = input
        .env_vars
        .iter()
        .map(|(k, v)| json!({ "name": k, "value": escape_k8s_expansion(v) }))
        .collect();

    let mut main = json!({
        "name": MAIN_CONTAINER,
        "image": input.image,
        "env": env,
        "workingDir": input.working_dir.to_string_lossy(),
        "volumeMounts": step_mounts,
    });
    // Docker semantics: `--entrypoint` replaces ENTRYPOINT, trailing args
    // replace CMD. Kubernetes spells those `command` and `args`.
    if let Some(ep) = input.entrypoint.filter(|s| !s.is_empty()) {
        main["command"] = json!([escape_k8s_expansion(ep)]);
    }
    if !input.cmd.is_empty() {
        main["args"] = json!(input
            .cmd
            .iter()
            .map(|arg| escape_k8s_expansion(arg))
            .collect::<Vec<_>>());
    }
    if !config.resources.is_empty() {
        main["resources"] = json!({
            "limits": config.resources,
            "requests": config.resources,
        });
    }

    let wait = json!({
        "name": WAIT_CONTAINER,
        "image": config.helper_image(),
        "command": ["sh", "-c", format!("until [ -f {} ]; do sleep 0.2; done", READY_MARKER)],
        "volumeMounts": helper_mounts,
        "resources": helper_resources(),
    });
    let sync = json!({
        "name": SYNC_CONTAINER,
        "image": config.helper_image(),
        "command": ["sh", "-c", "while true; do sleep 3600; done"],
        "volumeMounts": helper_mounts,
        "resources": helper_resources(),
    });

    let mut spec = json!({
        "restartPolicy": "Never",
        "automountServiceAccountToken": false,
        "terminationGracePeriodSeconds": 2,
        // The sidecar never exits on its own. If wrkflw dies, this is what
        // stops an orphaned pod from holding its node's GPUs forever.
        "activeDeadlineSeconds": active_deadline_seconds(config),
        "initContainers": [wait],
        "containers": [main, sync],
        "volumes": volumes,
    });
    if !config.node_selector.is_empty() {
        spec["nodeSelector"] = json!(config.node_selector);
    }
    if !config.image_pull_secrets.is_empty() {
        spec["imagePullSecrets"] = json!(config
            .image_pull_secrets
            .iter()
            .map(|name| json!({ "name": name }))
            .collect::<Vec<_>>());
    }

    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": input.name,
            "labels": {
                MANAGED_BY_LABEL: "wrkflw",
                SESSION_LABEL: *SESSION_ID,
            },
        },
        "spec": spec,
    })
}

/// `activeDeadlineSeconds` for `config`. The API field is a signed 64-bit
/// integer, and a wrapped sum would kill the pod during its own workspace sync.
pub(super) fn active_deadline_seconds(config: &KubernetesConfig) -> u64 {
    config
        .timeout()
        .as_secs()
        .saturating_add(DEADLINE_MARGIN_SECS)
        .min(i64::MAX as u64)
}

/// Requests and limits for the helper containers. They only run `sh` and
/// `tar`, but a namespace with a `ResourceQuota` rejects any container that
/// does not declare these, and there would be no flag to fix it with.
pub(super) fn helper_resources() -> Value {
    json!({
        "requests": { "cpu": "10m", "memory": "32Mi" },
        "limits": { "cpu": "500m", "memory": "256Mi" },
    })
}

/// Protect a string from Kubernetes' own variable expansion.
///
/// The API server treats `$(NAME)` as a reference and `$$` as an escaped `$`
/// in `command`, `args` and `env[].value`. Left alone, a script's `echo $$`
/// would reach the shell as `echo $`. Doubling every `$` makes the kubelet
/// hand the original string to the container.
pub(super) fn escape_k8s_expansion(s: &str) -> String {
    s.replace('$', "$$")
}

#[cfg(test)]
mod tests {
    use super::super::{DEFAULT_HELPER_IMAGE, DEFAULT_TIMEOUT_SECS};
    use super::*;

    fn cfg() -> KubernetesConfig {
        KubernetesConfig::default()
    }

    fn sample_mounts() -> Vec<Mount> {
        vec![
            Mount {
                host: PathBuf::from("/tmp/ws"),
                container: PathBuf::from("/github/workspace"),
            },
            Mount {
                host: PathBuf::from("/tmp/gh"),
                container: PathBuf::from("/github/workflow"),
            },
        ]
    }

    fn manifest_for(config: &KubernetesConfig, cmd: &[&str], entrypoint: Option<&str>) -> Value {
        let mounts = sample_mounts();
        build_pod_manifest(
            config,
            &PodInput {
                name: "wrkflw-test",
                image: "ubuntu:22.04",
                cmd,
                env_vars: &[("FOO", "bar")],
                working_dir: Path::new("/github/workspace/sub"),
                mounts: &mounts,
                entrypoint,
            },
        )
    }

    #[test]
    fn plan_mounts_sorts_parents_first_and_dedupes_container_paths() {
        let volumes = [
            (Path::new("/h/nested"), Path::new("/github/workspace/sub")),
            (Path::new("/h/ws"), Path::new("/github/workspace")),
            (Path::new("/h/other"), Path::new("/github/workspace")),
        ];
        let mounts = plan_mounts(&volumes);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].container, Path::new("/github/workspace"));
        assert_eq!(mounts[0].host, Path::new("/h/ws"), "first mapping wins");
        assert_eq!(mounts[1].container, Path::new("/github/workspace/sub"));
    }

    #[test]
    fn nested_excludes_lists_child_mounts_relative_to_parent() {
        let mounts = plan_mounts(&[
            (Path::new("/h/ws"), Path::new("/github/workspace")),
            (
                Path::new("/h/nested"),
                Path::new("/github/workspace/sub/deep"),
            ),
            (Path::new("/h/gh"), Path::new("/github/workflow")),
        ]);
        let by_path = |p: &str| {
            mounts
                .iter()
                .find(|m| m.container == Path::new(p))
                .expect("mount present")
        };
        assert_eq!(
            nested_excludes(by_path("/github/workspace"), &mounts),
            vec!["./sub/deep"]
        );
        assert!(nested_excludes(by_path("/github/workspace/sub/deep"), &mounts).is_empty());
        assert!(nested_excludes(by_path("/github/workflow"), &mounts).is_empty());
    }

    #[test]
    fn manifest_has_wait_init_container_step_and_sync_sidecar() {
        let m = manifest_for(&cfg(), &["sh", "-c", "echo hi"], None);
        assert_eq!(m["kind"], "Pod");
        assert_eq!(m["metadata"]["name"], "wrkflw-test");
        assert_eq!(m["metadata"]["labels"][MANAGED_BY_LABEL], "wrkflw");
        assert_eq!(m["metadata"]["labels"][SESSION_LABEL], *SESSION_ID);

        let spec = &m["spec"];
        assert_eq!(spec["restartPolicy"], "Never");
        assert_eq!(
            spec["activeDeadlineSeconds"],
            DEFAULT_TIMEOUT_SECS + DEADLINE_MARGIN_SECS
        );
        assert_eq!(spec["automountServiceAccountToken"], false);
        assert_eq!(spec["initContainers"][0]["name"], WAIT_CONTAINER);
        assert_eq!(spec["initContainers"][0]["image"], DEFAULT_HELPER_IMAGE);
        assert!(spec["initContainers"][0]["command"][2]
            .as_str()
            .unwrap()
            .contains(READY_MARKER));

        let containers = spec["containers"].as_array().unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], MAIN_CONTAINER);
        assert_eq!(containers[1]["name"], SYNC_CONTAINER);
        assert!(spec["nodeSelector"].is_null());
        assert!(spec["imagePullSecrets"].is_null());
    }

    #[test]
    fn manifest_step_container_carries_image_cmd_env_and_mounts() {
        let m = manifest_for(&cfg(), &["sh", "-c", "echo hi"], None);
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["image"], "ubuntu:22.04");
        assert_eq!(main["args"], json!(["sh", "-c", "echo hi"]));
        assert!(
            main["command"].is_null(),
            "ENTRYPOINT is kept unless overridden"
        );
        assert_eq!(main["workingDir"], "/github/workspace/sub");
        assert_eq!(main["env"], json!([{ "name": "FOO", "value": "bar" }]));
        assert!(main["resources"].is_null());

        let mounts = main["volumeMounts"].as_array().unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0]["mountPath"], "/github/workspace");
        assert_eq!(mounts[1]["mountPath"], "/github/workflow");
        // The step must not see the sync marker volume.
        assert!(mounts.iter().all(|v| v["mountPath"] != SYNC_DIR));

        // Helpers see every step mount plus the marker volume.
        let helper = m["spec"]["containers"][1]["volumeMounts"]
            .as_array()
            .unwrap();
        assert_eq!(helper.len(), 3);
        assert_eq!(helper[2]["mountPath"], SYNC_DIR);
        let wait = m["spec"]["initContainers"][0]["volumeMounts"]
            .as_array()
            .unwrap();
        assert_eq!(wait.len(), 3);

        let volumes = m["spec"]["volumes"].as_array().unwrap();
        assert_eq!(volumes.len(), 3);
        assert!(volumes.iter().all(|v| v["emptyDir"].is_object()));
    }

    #[test]
    fn manifest_maps_entrypoint_to_command_and_empty_cmd_to_image_defaults() {
        let m = manifest_for(&cfg(), &[], Some("/entrypoint.sh"));
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["command"], json!(["/entrypoint.sh"]));
        assert!(main["args"].is_null());

        let m = manifest_for(&cfg(), &[], None);
        let main = &m["spec"]["containers"][0];
        assert!(main["command"].is_null());
        assert!(main["args"].is_null());

        // An empty entrypoint string is "not set", as in the Docker backend.
        let m = manifest_for(&cfg(), &["ls"], Some(""));
        assert!(m["spec"]["containers"][0]["command"].is_null());
    }

    #[test]
    fn manifest_applies_node_selector_resources_pull_secrets_and_helper_image() {
        let mut config = cfg();
        config
            .node_selector
            .insert("accelerator".into(), "a100".into());
        config.resources.insert("nvidia.com/gpu".into(), "1".into());
        config.resources.insert("memory".into(), "8Gi".into());
        config.image_pull_secrets = vec!["regcred".into()];
        config.helper_image = Some("alpine:3.19".into());

        let m = manifest_for(&config, &["nvidia-smi"], None);
        let spec = &m["spec"];
        assert_eq!(spec["nodeSelector"], json!({ "accelerator": "a100" }));
        assert_eq!(spec["imagePullSecrets"], json!([{ "name": "regcred" }]));
        assert_eq!(spec["initContainers"][0]["image"], "alpine:3.19");
        assert_eq!(spec["containers"][1]["image"], "alpine:3.19");

        let resources = &spec["containers"][0]["resources"];
        assert_eq!(resources["limits"]["nvidia.com/gpu"], "1");
        assert_eq!(resources["requests"]["nvidia.com/gpu"], "1");
        assert_eq!(resources["limits"]["memory"], "8Gi");
        // Helpers never request accelerators, but do satisfy resource quotas.
        for helper in [&spec["initContainers"][0], &spec["containers"][1]] {
            assert_eq!(helper["resources"], helper_resources());
        }
    }

    #[test]
    fn active_deadline_never_wraps_or_overflows_the_api_field() {
        let with_timeout = |secs| KubernetesConfig {
            timeout_secs: Some(secs),
            ..Default::default()
        };
        assert_eq!(
            active_deadline_seconds(&with_timeout(120)),
            120 + DEADLINE_MARGIN_SECS
        );
        assert_eq!(
            active_deadline_seconds(&with_timeout(u64::MAX)),
            i64::MAX as u64
        );
    }

    #[test]
    fn is_nested_finds_mounts_inside_other_mounts() {
        let mounts = plan_mounts(&[
            (Path::new("/h/ws"), Path::new("/github/workspace")),
            (Path::new("/h/sub"), Path::new("/github/workspace/sub")),
            (Path::new("/h/gh"), Path::new("/github/workflow")),
        ]);
        let nested: Vec<_> = mounts
            .iter()
            .filter(|m| is_nested(m, &mounts))
            .map(|m| m.container.clone())
            .collect();
        assert_eq!(nested, vec![PathBuf::from("/github/workspace/sub")]);
    }

    #[test]
    fn manifest_serializes_as_valid_json_with_no_volumes() {
        let m = build_pod_manifest(
            &cfg(),
            &PodInput {
                name: "wrkflw-empty",
                image: "alpine",
                cmd: &["true"],
                env_vars: &[],
                working_dir: Path::new("/"),
                mounts: &[],
                entrypoint: None,
            },
        );
        let text = serde_json::to_string(&m).unwrap();
        assert!(text.contains("\"wrkflw-empty\""));
        assert_eq!(m["spec"]["volumes"].as_array().unwrap().len(), 1);
        assert!(m["spec"]["containers"][0]["volumeMounts"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn manifest_escapes_dollars_from_kubernetes_expansion() {
        assert_eq!(escape_k8s_expansion("no dollars"), "no dollars");
        assert_eq!(escape_k8s_expansion("$"), "$$");
        assert_eq!(escape_k8s_expansion("$$ $(HOME)"), "$$$$ $$(HOME)");

        let mounts = sample_mounts();
        let m = build_pod_manifest(
            &cfg(),
            &PodInput {
                name: "wrkflw-test",
                image: "ubuntu:22.04",
                cmd: &["bash", "-c", "echo $$ $(date)"],
                env_vars: &[("LIT", "a$(HOME)b")],
                working_dir: Path::new("/github/workspace"),
                mounts: &mounts,
                entrypoint: Some("/bin/$x"),
            },
        );
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["args"][2], "echo $$$$ $$(date)");
        assert_eq!(main["env"][0]["value"], "a$$(HOME)b");
        assert_eq!(main["command"][0], "/bin/$$x");
    }

    #[test]
    fn only_the_executors_own_directories_are_mirrored() {
        let mirrored = |container: &str| {
            Mount {
                host: PathBuf::from("/h"),
                container: PathBuf::from(container),
            }
            .mirrored()
        };
        assert!(mirrored("/github/workspace"));
        assert!(mirrored("/github/workflow"));
        assert!(!mirrored("/github/action"));
        assert!(!mirrored("/github/workspace/cache"));
        assert!(!mirrored("/github"));
        assert!(!mirrored("/tmp"));
    }

    #[test]
    fn plan_mounts_keeps_pod_only_volumes() {
        let mounts = plan_mounts(&[(Path::new(""), Path::new("/scratch"))]);
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].container, Path::new("/scratch"));
        assert!(mounts[0].host.as_os_str().is_empty());
    }

    #[test]
    fn plan_mounts_skips_files_but_keeps_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("docker.sock");
        std::fs::write(&file, "").unwrap();
        let missing = dir.path().join("not-yet");
        let mounts = plan_mounts(&[
            (file.as_path(), Path::new("/var/run/docker.sock")),
            (dir.path(), Path::new("/data")),
            (missing.as_path(), Path::new("/later")),
        ]);
        let paths: Vec<_> = mounts.iter().map(|m| m.container.clone()).collect();
        assert_eq!(paths, vec![PathBuf::from("/data"), PathBuf::from("/later")]);
    }
}
