#!/bin/bash
# Example script to trigger GitLab pipelines using wrkflw

# Check if GITLAB_TOKEN is set
if [ -z "${GITLAB_TOKEN}" ]; then
  echo "Error: GITLAB_TOKEN environment variable is not set."
  echo "Please set it with: export GITLAB_TOKEN=your_token_here"
  exit 1
fi

# Ensure we're in a Git repository
if ! git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
  echo "Error: Not in a Git repository."
  echo "Please run this script from within a Git repository with a GitLab remote."
  exit 1
fi

# Check for .gitlab-ci.yml file
if [ ! -f .gitlab-ci.yml ]; then
  echo "Warning: No .gitlab-ci.yml file found in the current directory."
  echo "The pipeline trigger might fail if there is no pipeline configuration."
fi

# Function to display help
show_help() {
  echo "GitLab Pipeline Trigger Examples"
  echo "--------------------------------"
  echo "Usage: $0 [example-number]"
  echo ""
  echo "Available examples:"
  echo "  1: Trigger default pipeline on the current branch"
  echo "  2: Trigger pipeline on main branch"
  echo "  3: Trigger release build"
  echo "  4: Trigger documentation build"
  echo "  5: Trigger pipeline with multiple variables"
  echo ""
  echo "For custom commands, modify this script or run wrkflw directly:"
  echo "  wrkflw trigger-gitlab [options]"
}

# No arguments, show help
if [ $# -eq 0 ]; then
  show_help
  exit 0
fi

# Handle examples
case "$1" in
  "1")
    echo "Triggering default pipeline on the current branch..."
    wrkflw trigger-gitlab
    ;;
  
  "2")
    echo "Triggering pipeline on main branch..."
    wrkflw trigger-gitlab --branch main
    ;;
  
  "3")
    echo "Triggering release build..."
    wrkflw trigger-gitlab --variable BUILD_RELEASE=true
    ;;
  
  "4")
    echo "Triggering documentation build..."
    wrkflw trigger-gitlab --variable BUILD_DOCS=true
    ;;
  
  "5")
    echo "Triggering pipeline with multiple variables..."
    wrkflw trigger-gitlab --variable BUILD_RELEASE=true --variable BUILD_DOCS=true
    ;;
  
  *)
    echo "Unknown example: $1"
    show_help
    exit 1
    ;;
esac 