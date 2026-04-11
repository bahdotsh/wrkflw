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
use crate::{is_gitlab_pipeline, RuntimeChoice};
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

    // Evaluate trigger filter at the call site before executing
    if ctx.diff || ctx.event.is_some() || ctx.changed_files.is_some() {
        if is_gitlab {
            eprintln!(
                "Error: --diff, --event, and --changed-files are only \
                 supported for GitHub Actions workflows.\n\
                 {} appears to be a GitLab CI pipeline — trigger \
                 filtering is GitHub Actions-specific and cannot be \
                 evaluated against GitLab `rules:`/`only:`/`except:`.",
                ctx.path.display()
            );
            std::process::exit(1);
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
            Ok(prefilter::PrefilterDecision::Proceed) => {
                // Match — fall through to the executor below.
            }
            Ok(prefilter::PrefilterDecision::Skip { reason }) => {
                use wrkflw_ui::cli_style;
                println!(
                    "{}",
                    cli_style::dim(&format!("Workflow skipped: {}", reason))
                );
                std::process::exit(0);
            }
            Err(msg) => {
                eprintln!("Error: {}", msg);
                std::process::exit(1);
            }
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
    let result = wrkflw_executor::execute_workflow(&ctx.path, config)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error executing workflow: {}", e);
            std::process::exit(1);
        });

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
