pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<String>,
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationResult {
    pub fn new() -> Self {
        ValidationResult {
            is_valid: true,
            issues: Vec::new(),
        }
    }

    pub fn add_issue(&mut self, issue: String) {
        self.is_valid = false;
        self.issues.push(issue);
    }
}

// GitLab pipeline models
pub mod gitlab {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    /// Represents a GitLab CI/CD pipeline configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Pipeline {
        /// Default image for all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image: Option<Image>,

        /// Global variables available to all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub variables: Option<HashMap<String, String>>,

        /// Pipeline stages in execution order
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stages: Option<Vec<String>>,

        /// Default before_script for all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub before_script: Option<Vec<String>>,

        /// Default after_script for all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub after_script: Option<Vec<String>>,

        /// Job definitions (name => job)
        #[serde(flatten)]
        pub jobs: HashMap<String, Job>,

        /// Workflow rules for the pipeline
        #[serde(skip_serializing_if = "Option::is_none")]
        pub workflow: Option<Workflow>,

        /// Includes for pipeline configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub include: Option<Vec<Include>>,
    }

    /// A job in a GitLab CI/CD pipeline
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Job {
        /// The stage this job belongs to
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stage: Option<String>,

        /// Docker image to use for this job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image: Option<Image>,

        /// Script commands to run
        #[serde(skip_serializing_if = "Option::is_none")]
        pub script: Option<Vec<String>>,

        /// Commands to run before the main script
        #[serde(skip_serializing_if = "Option::is_none")]
        pub before_script: Option<Vec<String>>,

        /// Commands to run after the main script
        #[serde(skip_serializing_if = "Option::is_none")]
        pub after_script: Option<Vec<String>>,

        /// When to run the job (on_success, on_failure, always, manual)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,

        /// Allow job failure
        #[serde(skip_serializing_if = "Option::is_none")]
        pub allow_failure: Option<bool>,

        /// Services to run alongside the job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub services: Option<Vec<Service>>,

        /// Tags to define which runners can execute this job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tags: Option<Vec<String>>,

        /// Job-specific variables
        #[serde(skip_serializing_if = "Option::is_none")]
        pub variables: Option<HashMap<String, String>>,

        /// Job dependencies
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dependencies: Option<Vec<String>>,

        /// Artifacts to store after job execution
        #[serde(skip_serializing_if = "Option::is_none")]
        pub artifacts: Option<Artifacts>,

        /// Cache configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache: Option<Cache>,

        /// Rules for when this job should run
        #[serde(skip_serializing_if = "Option::is_none")]
        pub rules: Option<Vec<Rule>>,

        /// Only run on specified refs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub only: Option<Only>,

        /// Exclude specified refs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub except: Option<Except>,

        /// Retry configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub retry: Option<Retry>,

        /// Timeout for the job in seconds
        #[serde(skip_serializing_if = "Option::is_none")]
        pub timeout: Option<String>,

        /// Mark job as parallel and specify instance count
        #[serde(skip_serializing_if = "Option::is_none")]
        pub parallel: Option<usize>,

        /// Flag to indicate this is a template job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub template: Option<bool>,

        /// List of jobs this job extends from
        #[serde(skip_serializing_if = "Option::is_none")]
        pub extends: Option<Vec<String>>,
    }

    /// Docker image configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Image {
        /// Simple image name as string
        Simple(String),
        /// Detailed image configuration
        Detailed {
            /// Image name
            name: String,
            /// Entrypoint to override in the image
            #[serde(skip_serializing_if = "Option::is_none")]
            entrypoint: Option<Vec<String>>,
        },
    }

    /// Service container to run alongside a job
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Service {
        /// Simple service name as string
        Simple(String),
        /// Detailed service configuration
        Detailed {
            /// Service name/image
            name: String,
            /// Command to run in the service container
            #[serde(skip_serializing_if = "Option::is_none")]
            command: Option<Vec<String>>,
            /// Entrypoint to override in the image
            #[serde(skip_serializing_if = "Option::is_none")]
            entrypoint: Option<Vec<String>>,
        },
    }

    /// Artifacts configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Artifacts {
        /// Paths to include as artifacts
        #[serde(skip_serializing_if = "Option::is_none")]
        pub paths: Option<Vec<String>>,
        /// Artifact expiration duration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub expire_in: Option<String>,
        /// When to upload artifacts (on_success, on_failure, always)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
    }

    /// Cache configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Cache {
        /// Cache key
        #[serde(skip_serializing_if = "Option::is_none")]
        pub key: Option<String>,
        /// Paths to cache
        #[serde(skip_serializing_if = "Option::is_none")]
        pub paths: Option<Vec<String>>,
        /// When to save cache (on_success, on_failure, always)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
        /// Cache policy
        #[serde(skip_serializing_if = "Option::is_none")]
        pub policy: Option<String>,
    }

    /// Rule for conditional job execution
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Rule {
        /// If condition expression
        #[serde(skip_serializing_if = "Option::is_none")]
        pub if_: Option<String>,
        /// When to run if condition is true
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
        /// Variables to set if condition is true
        #[serde(skip_serializing_if = "Option::is_none")]
        pub variables: Option<HashMap<String, String>>,
    }

    /// Only/except configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Only {
        /// Simple list of refs
        Refs(Vec<String>),
        /// Detailed configuration
        Complex {
            /// Refs to include
            #[serde(skip_serializing_if = "Option::is_none")]
            refs: Option<Vec<String>>,
            /// Branch patterns to include
            #[serde(skip_serializing_if = "Option::is_none")]
            branches: Option<Vec<String>>,
            /// Tags to include
            #[serde(skip_serializing_if = "Option::is_none")]
            tags: Option<Vec<String>>,
            /// Pipeline types to include
            #[serde(skip_serializing_if = "Option::is_none")]
            variables: Option<Vec<String>>,
            /// Changes to files that trigger the job
            #[serde(skip_serializing_if = "Option::is_none")]
            changes: Option<Vec<String>>,
        },
    }

    /// Except configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Except {
        /// Simple list of refs
        Refs(Vec<String>),
        /// Detailed configuration
        Complex {
            /// Refs to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            refs: Option<Vec<String>>,
            /// Branch patterns to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            branches: Option<Vec<String>>,
            /// Tags to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            tags: Option<Vec<String>>,
            /// Pipeline types to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            variables: Option<Vec<String>>,
            /// Changes to files that don't trigger the job
            #[serde(skip_serializing_if = "Option::is_none")]
            changes: Option<Vec<String>>,
        },
    }

    /// Workflow configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Workflow {
        /// Rules for when to run the pipeline
        pub rules: Vec<Rule>,
    }

    /// Retry configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Retry {
        /// Simple max attempts
        MaxAttempts(u32),
        /// Detailed retry configuration
        Detailed {
            /// Maximum retry attempts
            max: u32,
            /// When to retry
            #[serde(skip_serializing_if = "Option::is_none")]
            when: Option<Vec<String>>,
        },
    }

    /// Include configuration for external pipeline files
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Include {
        /// Simple string include
        Local(String),
        /// Detailed include configuration
        Detailed {
            /// Local file path
            #[serde(skip_serializing_if = "Option::is_none")]
            local: Option<String>,
            /// Remote file URL
            #[serde(skip_serializing_if = "Option::is_none")]
            remote: Option<String>,
            /// Include from project
            #[serde(skip_serializing_if = "Option::is_none")]
            project: Option<String>,
            /// Include specific file from project
            #[serde(skip_serializing_if = "Option::is_none")]
            file: Option<String>,
            /// Include template
            #[serde(skip_serializing_if = "Option::is_none")]
            template: Option<String>,
            /// Ref to use when including from project
            #[serde(skip_serializing_if = "Option::is_none")]
            ref_: Option<String>,
        },
    }
}
