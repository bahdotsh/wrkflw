# Using wrkflw with GitLab Pipelines

This guide explains how to use the `wrkflw` tool to trigger GitLab CI/CD pipelines.

## Prerequisites

1. A GitLab repository with a `.gitlab-ci.yml` file
2. A GitLab personal access token with API access
3. `wrkflw` installed on your system

## Setting Up

1. Create a GitLab personal access token:
   - Go to GitLab > User Settings > Access Tokens
   - Create a token with `api` scope
   - Copy the token value

2. Set the token as an environment variable:
   ```bash
   export GITLAB_TOKEN=your_token_here
   ```

## Triggering a Pipeline

You can trigger a GitLab pipeline using the `trigger-gitlab` command:

```bash
# Trigger using the default branch
wrkflw trigger-gitlab

# Trigger on a specific branch
wrkflw trigger-gitlab --branch feature-branch

# Trigger with custom variables
wrkflw trigger-gitlab --variable BUILD_RELEASE=true
```

### Example: Triggering a Release Build

To trigger the release build job in our sample pipeline:

```bash
wrkflw trigger-gitlab --variable BUILD_RELEASE=true
```

This will set the `BUILD_RELEASE` variable to `true`, which activates the release job in our sample pipeline.

### Example: Building Documentation

To trigger the documentation build job:

```bash
wrkflw trigger-gitlab --variable BUILD_DOCS=true
```

## Controlling Job Execution with Variables

Our sample GitLab pipeline is configured to make certain jobs conditional based on variables. You can use the `--variable` flag to control which jobs run:

| Variable | Purpose |
|----------|---------|
| `BUILD_RELEASE` | Set to `true` to run the release job |
| `BUILD_DOCS` | Set to `true` to build documentation |

## Checking Pipeline Status

After triggering a pipeline, you can check its status directly on GitLab:

1. Navigate to your GitLab repository
2. Go to CI/CD > Pipelines
3. Find your recently triggered pipeline

The `wrkflw` command will also provide a direct URL to the pipeline after triggering.

## Troubleshooting

If you encounter issues:

1. Verify your GitLab token is set correctly
2. Check that you're in a repository with a valid GitLab remote URL
3. Ensure your `.gitlab-ci.yml` file is valid
4. Check that your GitLab token has API access permissions
5. Review GitLab's CI/CD pipeline logs for detailed error information 