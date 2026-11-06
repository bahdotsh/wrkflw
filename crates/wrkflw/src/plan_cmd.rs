//! `wrkflw plan`: what a run would do, without running it.

use crate::{is_gitlab_pipeline, prefilter, OutputFormat, RuntimeChoice};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wrkflw_executor::runner_fidelity::{requested_os, runner_os_for, RequestedOs};
use wrkflw_parser::workflow::{Job, WorkflowDefinition};
use wrkflw_runs::SCHEMA_VERSION;
use wrkflw_secrets::{SecretError, SecretManager, SecretSubstitution};

pub(crate) struct PlanCtx {
    pub(crate) path: PathBuf,
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
    pub(crate) format: OutputFormat,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) platform_overrides: HashMap<String, String>,
}

/// The trigger decision, or `null` when no event context was given.
async fn trigger(ctx: &PlanCtx) -> Result<Value, String> {
    if !(ctx.diff || ctx.event.is_some() || ctx.changed_files.is_some()) {
        return Ok(Value::Null);
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
        strict_filter: prefilter::effective_strict_filter(ctx.strict_filter, ctx.no_strict_filter),
    })
    .await?;
    let event = ctx.event.clone().unwrap_or_else(|| "push".to_string());
    Ok(match decision {
        prefilter::PrefilterDecision::Proceed { reason, .. } => {
            json!({"event": event, "matches": true, "reason": reason})
        }
        prefilter::PrefilterDecision::Skip { reason } => {
            json!({"event": event, "matches": false, "reason": reason})
        }
    })
}

/// Secrets the workflow reads that no provider has. They expand to nothing
/// in a run, which is worth knowing before the run fails for it.
async fn missing_secrets(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Round-tripped through YAML so that commented-out lines don't count.
    let parsed: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
    let text = serde_yaml::to_string(&parsed).map_err(|e| e.to_string())?;
    let manager = SecretManager::default().await.map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    for secret in SecretSubstitution::extract_secret_refs(&text) {
        let name = secret.qualified_name();
        if matches!(
            secret.lookup(&manager).await,
            Err(SecretError::NotFound { .. })
        ) && !missing.contains(&name)
        {
            missing.push(name);
        }
    }
    missing.sort();
    Ok(missing)
}

/// Expanded matrix legs. A matrix computed at run time can't be expanded yet,
/// and saying so beats guessing.
fn matrix_json(job: &Job) -> Value {
    let Some(config) = job.matrix_config() else {
        return Value::Null;
    };
    if config.expression.is_some() {
        return json!({"dynamic": true, "combinations": []});
    }
    match wrkflw_matrix::expand_matrix(config) {
        Ok(combinations) => {
            let values: Vec<&_> = combinations.iter().map(|c| &c.values).collect();
            json!({"dynamic": false, "combinations": values})
        }
        // Axes with `${{ }}` only resolve once upstream jobs have run.
        Err(e) => json!({"dynamic": true, "combinations": [], "note": e.to_string()}),
    }
}

fn job_json(id: &str, job: &Job) -> Value {
    let steps: Vec<Value> = job
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            json!({
                "index": index,
                "id": step.id,
                "name": step.name,
                "uses": step.uses,
                "run": step.run,
                "if": step.if_condition,
            })
        })
        .collect();
    json!({
        "id": id,
        "needs": job.needs.clone().unwrap_or_default(),
        "runs_on": job.runs_on,
        "if": job.if_condition,
        "uses": job.uses,
        "matrix": matrix_json(job),
        "steps": steps,
    })
}

/// The OS each of `job`'s matrix legs, or the job itself, asks for, under
/// the name a run gives it. A leg of a matrix computed at run time can't be
/// named yet, so it is left out.
fn requested(
    id: &str,
    job: &Job,
    overrides: &HashMap<String, String>,
) -> Vec<(String, RequestedOs)> {
    let Some(config) = job.matrix_config() else {
        return requested_os(job, None, overrides)
            .map(|os| (id.to_string(), os))
            .into_iter()
            .collect();
    };
    if config.expression.is_some() {
        return Vec::new();
    }
    wrkflw_matrix::expand_matrix(config)
        .unwrap_or_default()
        .iter()
        .filter_map(|leg| {
            let os = requested_os(job, Some(&leg.values), overrides)?;
            Some((wrkflw_matrix::format_combination_name(id, leg), os))
        })
        .collect()
}

/// The jobs, or matrix legs, that would run as another OS than `runs-on`
/// asks for, in the shape `run_finished` reports them.
fn unverified(requests: &[(String, RequestedOs)], runtime: RuntimeChoice) -> Vec<Value> {
    let ran_as = runner_os_for(runtime.into());
    requests
        .iter()
        .filter_map(|(name, request)| Some(request.ran_as(&ran_as)?.unverified_entry(name)))
        .collect()
}

type GithubPlan = (Vec<Vec<String>>, Vec<Value>, Vec<Value>);

fn github_plan(ctx: &PlanCtx, workflow: &WorkflowDefinition) -> Result<GithubPlan, String> {
    let mut batches = wrkflw_executor::dependency::resolve_dependencies(workflow)?;
    if let Some(target) = &ctx.job {
        batches = wrkflw_executor::dependency::filter_plan_to_job(
            batches,
            target,
            &workflow.jobs,
            "workflow",
        )?;
    }
    let mut jobs = Vec::new();
    let mut requests = Vec::new();
    for batch in &batches {
        for id in batch {
            if let Some(job) = workflow.jobs.get(id) {
                jobs.push(job_json(id, job));
                requests.extend(requested(id, job, &ctx.platform_overrides));
            }
        }
    }
    let substituted = unverified(&requests, ctx.runtime.clone());
    Ok((batches, jobs, substituted))
}

fn gitlab_plan(ctx: &PlanCtx) -> Result<(Vec<Vec<String>>, Vec<Value>), String> {
    let pipeline = wrkflw_parser::gitlab::parse_pipeline(&ctx.path).map_err(|e| e.to_string())?;
    let stages = pipeline.stages.clone().unwrap_or_else(|| {
        ["build", "test", "deploy"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    });
    let mut batches = Vec::new();
    let mut jobs = Vec::new();
    for stage in stages {
        let mut names: Vec<String> = pipeline
            .jobs
            .iter()
            .filter(|(name, job)| {
                !name.starts_with('.')
                    && job.stage.as_deref().unwrap_or("test") == stage
                    && ctx.job.as_ref().is_none_or(|target| target == *name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        if names.is_empty() {
            continue;
        }
        for name in &names {
            let job = &pipeline.jobs[name];
            jobs.push(json!({"id": name, "stage": stage, "script": job.script}));
        }
        batches.push(names);
    }
    Ok((batches, jobs))
}

async fn build(ctx: &PlanCtx) -> Result<Value, String> {
    if !ctx.path.is_file() {
        return Err(format!("workflow file not found: {}", ctx.path.display()));
    }
    let is_gitlab = ctx.gitlab || is_gitlab_pipeline(&ctx.path);
    if is_gitlab && (ctx.diff || ctx.event.is_some() || ctx.changed_files.is_some()) {
        return Err(
            "--diff, --event and --changed-files only apply to GitHub Actions workflows"
                .to_string(),
        );
    }
    let trigger = trigger(ctx).await?;
    let missing = if is_gitlab {
        Vec::new()
    } else {
        missing_secrets(&ctx.path).await?
    };
    let (name, (batches, jobs, unverified)) = if is_gitlab {
        let (batches, jobs) = gitlab_plan(ctx)?;
        (None, (batches, jobs, Vec::new()))
    } else {
        let workflow = wrkflw_parser::workflow::parse_workflow(&ctx.path)?;
        let plan = github_plan(ctx, &workflow)?;
        (Some(workflow.name.clone()), plan)
    };
    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "type": "plan",
        "workflow": ctx.path,
        "kind": if is_gitlab { "gitlab" } else { "github" },
        "name": name,
        "trigger": trigger,
        "would_run": trigger.is_null() || trigger["matches"] == true,
        "batches": batches,
        "jobs": jobs,
        "missing_secrets": missing,
        "unverified": unverified,
    }))
}

fn print_text(plan: &Value) {
    use wrkflw_ui::cli_style;
    println!(
        "{}",
        cli_style::section(&format!(
            "Plan for {}",
            plan["workflow"].as_str().unwrap_or("")
        ))
    );
    if let Some(reason) = plan["trigger"]["reason"].as_str() {
        let verdict = if plan["trigger"]["matches"] == true {
            "fires"
        } else {
            "does not fire"
        };
        println!(
            "Trigger: {} on {} ({})",
            verdict,
            plan["trigger"]["event"].as_str().unwrap_or(""),
            reason
        );
    }
    let missing: Vec<&str> = plan["missing_secrets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !missing.is_empty() {
        println!("Missing secrets, empty in a run: {}", missing.join(", "));
    }
    for entry in plan["unverified"].as_array().into_iter().flatten() {
        println!(
            "{}",
            cli_style::warning(&format!(
                "{} asks for {} but would run as {}: OS-specific steps won't be checked",
                entry["job"].as_str().unwrap_or(""),
                entry["requested"].as_str().unwrap_or(""),
                entry["ran_as"].as_str().unwrap_or(""),
            ))
        );
    }
    let batches = plan["batches"].as_array().cloned().unwrap_or_default();
    for (index, batch) in batches.iter().enumerate() {
        let names: Vec<&str> = batch
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        println!("Batch {}: {}", index + 1, names.join(", "));
    }
    for job in plan["jobs"].as_array().into_iter().flatten() {
        println!("\n{}", job["id"].as_str().unwrap_or(""));
        if let Some(condition) = job["if"].as_str() {
            println!("  if: {condition}");
        }
        match job["matrix"]["combinations"].as_array() {
            Some(_) if job["matrix"]["dynamic"] == true => {
                println!("  matrix: computed at run time")
            }
            Some(legs) => println!("  matrix: {} combination(s)", legs.len()),
            None => {}
        }
        for step in job["steps"].as_array().into_iter().flatten() {
            let label = step["name"]
                .as_str()
                .or(step["uses"].as_str())
                .unwrap_or("run");
            println!("  {}. {}", step["index"].as_u64().unwrap_or(0) + 1, label);
        }
    }
}

pub(crate) async fn run(ctx: PlanCtx) {
    match build(&ctx).await {
        Ok(plan) if ctx.format.is_json() => wrkflw_runs::emit_line(&plan),
        Ok(plan) => print_text(&plan),
        Err(message) => {
            if ctx.format.is_json() {
                wrkflw_runs::emit_line(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "error",
                    "message": message,
                }));
            }
            eprintln!("Error: {message}");
            std::process::exit(1);
        }
    }
}
