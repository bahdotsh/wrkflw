//! `wrkflw run` command orchestration.
//!
//! Extracted from `main.rs` so the single-workflow execution path
//! (prefilter → execute → print summary) lives next to its sibling
//! `watch_cmd` module instead of inlined into the 2k-line CLI entry
//! point. The function still owns its `std::process::exit` calls
//! because every exit site is tied to a specific already-formatted
//! CLI diagnostic — lifting them out to `main()` as a
//! `Result<(), String>` would force us to rewrite the "Workflow
//! execution failed:" block's colorized output into a flat error
//! string and regress user-visible output. The *extraction* is the
//! win here; further lifting of `exit` calls is a separate follow-up.

use crate::prefilter;
use crate::{is_gitlab_pipeline, OutputFormat, RuntimeChoice};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// Owned copy of the clap `Commands::Run` variant fields plus the
/// global `--verbose` flag. Built by `main()` from the match arm's
/// borrowed fields so the command body doesn't care about clap's
/// lifetime shape.
pub(crate) struct RunCtx {
    pub(crate) path: PathBuf,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) show_action_messages: bool,
    pub(crate) preserve_containers_on_failure: bool,
    pub(crate) gitlab: bool,
    pub(crate) job: Option<String>,
    pub(crate) variables: HashMap<String, String>,
    pub(crate) event: Option<String>,
    pub(crate) diff: bool,
    pub(crate) changed_files: Option<Vec<String>>,
    pub(crate) diff_base: Option<String>,
    pub(crate) diff_head: Option<String>,
    pub(crate) base_branch: Option<String>,
    pub(crate) activity_type: Option<String>,
    pub(crate) strict_filter: bool,
    pub(crate) no_strict_filter: bool,
    pub(crate) verbose: bool,
    pub(crate) platform_overrides: HashMap<String, String>,
    pub(crate) kubernetes: wrkflw_executor::KubernetesConfig,
    pub(crate) format: OutputFormat,
    pub(crate) intent: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) no_record: bool,
    pub(crate) from_step: Option<String>,
}

/// Print a terminal error in the active format and exit 1.
fn fail(format: OutputFormat, message: &str) -> ! {
    if format.is_json() {
        wrkflw_runs::emit_line(&json!({
            "schema_version": wrkflw_runs::SCHEMA_VERSION,
            "type": "error",
            "message": message,
        }));
    }
    eprintln!("Error: {}", message);
    std::process::exit(1);
}

/// `--from-step` as the executor wants it, after checking the job and step
/// exist, so a typo fails here rather than silently running every step.
fn resolve_from_step(
    ctx: &RunCtx,
    is_gitlab: bool,
) -> Result<Option<wrkflw_executor::FromStep>, String> {
    let Some(step) = ctx.from_step.clone() else {
        return Ok(None);
    };
    if is_gitlab {
        return Err("--from-step is only supported for GitHub Actions workflows".to_string());
    }
    let job = ctx.job.clone().unwrap_or_default();
    let workflow = wrkflw_parser::workflow::parse_workflow(&ctx.path)?;
    let definition = workflow
        .jobs
        .get(&job)
        .ok_or_else(|| format!("job '{}' not found in {}", job, ctx.path.display()))?;
    let from = wrkflw_executor::FromStep { job, step };
    if from.resolve(&definition.steps).is_none() {
        return Err(format!(
            "step '{}' not found in job '{}': pass a step name or a 1-based position",
            from.step, from.job
        ));
    }
    wrkflw_logging::warning(
        "--from-step: earlier steps are skipped, so their outputs and any files they \
         created are missing. Only actions/checkout still runs.",
    );
    Ok(Some(from))
}

/// Arguments of this invocation with `--variable` values removed, for replay.
/// Variables are often credentials, so they never reach the run store.
pub(crate) fn replayable_argv() -> Vec<String> {
    strip_variable_args(std::env::args().skip(1))
}

fn strip_variable_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        // `--format` is the replayer's choice, and clap rejects it twice.
        if arg == "-V" || arg == "--variable" || arg == "--format" {
            args.next();
        } else if !arg.starts_with("--variable=")
            && !arg.starts_with("--format=")
            && !(arg.starts_with("-V") && arg.len() > 2)
        {
            kept.push(arg);
        }
    }
    kept
}

/// Execute the `wrkflw run` command. Exits the process on every
/// terminal outcome (skip, prefilter error, executor error, workflow
/// failure). Returns normally on a successful run so the top-level
/// `main()` falls through to its existing cleanup path.
pub(crate) async fn run(ctx: RunCtx) {
    let strict_filter = prefilter::effective_strict_filter(ctx.strict_filter, ctx.no_strict_filter);
    // Determine workflow type up front so the trigger prefilter
    // can short-circuit for GitLab pipelines with a clear error.
    // Previously the prefilter ran first and tried to parse the
    // file as a GitHub workflow, which surfaced a confusing
    // `Error parsing workflow: ...` from deep in the YAML parser.
    let is_gitlab = ctx.gitlab || is_gitlab_pipeline(&ctx.path);

    let format = ctx.format;
    let from_step = resolve_from_step(&ctx, is_gitlab).unwrap_or_else(|e| fail(format, &e));

    if !is_gitlab && !ctx.variables.is_empty() {
        fail(
            format,
            "--variable is only supported for GitLab CI pipelines.",
        );
    }

    let mut variable_names: Vec<String> = ctx.variables.keys().cloned().collect();
    variable_names.sort();
    let mut meta = wrkflw_runs::RunMeta::new(if std::env::var_os("WRKFLW_MCP").is_some() {
        "mcp"
    } else {
        "cli"
    });
    meta.agent = ctx.agent.clone().or(meta.agent);
    meta.intent = ctx.intent.clone();
    meta.event = ctx.event.clone();
    meta.argv = replayable_argv();
    meta.variables = variable_names;
    let options = wrkflw_runs::RunOptions {
        meta,
        ndjson: format.is_json(),
        record: !ctx.no_record,
        from_step,
    };

    // Evaluate trigger filter at the call site before executing
    if ctx.diff || ctx.event.is_some() || ctx.changed_files.is_some() {
        if is_gitlab {
            fail(
                format,
                &format!(
                    "--diff, --event, and --changed-files are only \
                     supported for GitHub Actions workflows.\n\
                     {} appears to be a GitLab CI pipeline — trigger \
                     filtering is GitHub Actions-specific and cannot be \
                     evaluated against GitLab `rules:`/`only:`/`except:`.",
                    ctx.path.display()
                ),
            );
        }
        let decision = prefilter::run_trigger_prefilter(prefilter::PrefilterRequest {
            workflow_path: &ctx.path,
            event: ctx.event.as_ref(),
            diff: ctx.diff,
            changed_files: ctx.changed_files.as_ref(),
            diff_base: ctx.diff_base.as_deref(),
            diff_head: ctx.diff_head.as_ref(),
            base_branch: ctx.base_branch.as_ref(),
            activity_type: ctx.activity_type.as_ref(),
            verbose: ctx.verbose,
            strict_filter,
        })
        .await;
        match decision {
            Ok(prefilter::PrefilterDecision::Proceed { .. }) => {
                // Match — fall through to the executor below.
            }
            Ok(prefilter::PrefilterDecision::Skip { reason }) => {
                use wrkflw_ui::cli_style;
                let config = wrkflw_executor::ExecutionConfig {
                    runtime_type: ctx.runtime.into(),
                    ..Default::default()
                };
                wrkflw_runs::record_skipped(&ctx.path, &config, &options, &reason).await;
                if !format.is_json() {
                    println!(
                        "{}",
                        cli_style::dim(&format!("Workflow skipped: {}", reason))
                    );
                }
                std::process::exit(0);
            }
            Err(msg) => fail(format, &msg),
        }
    }

    // Create execution configuration
    let config = wrkflw_executor::ExecutionConfig {
        runtime_type: ctx.runtime.into(),
        verbose: ctx.verbose,
        preserve_containers_on_failure: ctx.preserve_containers_on_failure,
        secrets_config: None, // Use default secrets configuration
        show_action_messages: ctx.show_action_messages,
        target_job: ctx.job.clone(),
        platform_overrides: ctx.platform_overrides,
        kubernetes: ctx.kubernetes,
        variables: ctx.variables,
    };
    let workflow_type = if is_gitlab {
        "GitLab CI pipeline"
    } else {
        "GitHub workflow"
    };

    wrkflw_logging::info(&format!(
        "Running {} at: {}",
        workflow_type,
        ctx.path.display()
    ));

    // Execute the workflow
    let recorded = wrkflw_runs::execute_recorded(&ctx.path, config, options).await;
    let digest = recorded.digest;
    let result = match recorded.result {
        Ok(result) => result,
        // In JSON mode `run_finished` has already carried the error too.
        Err(e) => {
            eprintln!("Error executing workflow: {}", e);
            std::process::exit(1);
        }
    };

    if format.is_json() {
        if result.failure_details.is_some() {
            std::process::exit(1);
        }
        return;
    }

    // Print execution summary
    use wrkflw_ui::cli_style;
    if result.failure_details.is_some() {
        eprintln!("{}", cli_style::error("Workflow execution failed:"));
        if let Some(details) = result.failure_details {
            if ctx.verbose {
                eprintln!("{}", details);
            } else {
                let simplified_error = details
                    .lines()
                    .filter(|line| {
                        line.contains(wrkflw_logging::symbols::FAILURE)
                            || line.trim().starts_with("Error:")
                    })
                    .take(5)
                    .collect::<Vec<&str>>()
                    .join("\n");

                eprintln!("{}", simplified_error);

                if details.lines().count() > 5 {
                    eprintln!(
                        "\n{}",
                        cli_style::dim("Use --verbose flag to see full error details")
                    );
                }
            }
        }
        if let Some(digest) = &digest {
            eprintln!("\n{}", cli_style::section("Failure digest"));
            eprint!("{}", wrkflw_runs::render_digest(digest));
        }
        std::process::exit(1);
    } else {
        println!(
            "{}",
            cli_style::success("Workflow execution completed successfully!")
        );

        println!("{}", cli_style::section("Job summary"));
        for job in result.jobs {
            match job.status {
                wrkflw_executor::JobStatus::Success => {
                    println!("  {}", cli_style::job_success(&job.name))
                }
                wrkflw_executor::JobStatus::Failure if job.failure_allowed => {
                    println!("  {}", cli_style::job_failure_allowed(&job.name))
                }
                wrkflw_executor::JobStatus::Failure => {
                    println!("  {}", cli_style::job_failure(&job.name))
                }
                wrkflw_executor::JobStatus::Skipped => {
                    println!("  {}", cli_style::job_skipped(&job.name))
                }
            }

            for step in job.steps {
                match step.status {
                    wrkflw_executor::StepStatus::Success => {
                        println!("{}", cli_style::step_success(&step.name))
                    }
                    wrkflw_executor::StepStatus::Failure => {
                        println!("{}", cli_style::step_failure(&step.name));

                        if !ctx.verbose {
                            let error_lines = step
                                .output
                                .lines()
                                .filter(|line| {
                                    line.contains("error:")
                                        || line.contains("Error:")
                                        || line.trim().starts_with("Exit code:")
                                        || line.contains("failed")
                                })
                                .take(3)
                                .collect::<Vec<&str>>();

                            if !error_lines.is_empty() {
                                for line in error_lines {
                                    println!("{}", cli_style::indent(line.trim()));
                                }

                                if step.output.lines().count() > 3 {
                                    println!(
                                        "{}",
                                        cli_style::indent("(Use --verbose for full output)")
                                    );
                                }
                            }
                        }
                    }
                    wrkflw_executor::StepStatus::Skipped => {
                        println!("{}", cli_style::step_skipped(&step.name))
                    }
                }
            }
        }
    }

    // Cleanup is handled automatically via the signal handler
}

#[cfg(test)]
mod tests {
    use super::strip_variable_args;

    #[test]
    fn variable_values_never_reach_the_stored_argv() {
        let args = [
            "run",
            "ci.yml",
            "-V",
            "TOKEN=a",
            "--variable",
            "B=b",
            "--variable=C=c",
            "-VD=d",
            "--format",
            "json",
            "--format=json",
            "--job",
            "build",
        ];
        let kept = strip_variable_args(args.iter().map(|s| s.to_string()));
        assert_eq!(kept, vec!["run", "ci.yml", "--job", "build"]);
    }
}
