//! Writes `docker/runner.Dockerfile` and `docker/runner-image.txt`, the
//! runner base image the release workflow publishes, from what this
//! wrkflw would build. Run it after changing what every job image gets.

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dockerfile = wrkflw_executor::engine::runner_base_dockerfile();
    let image = wrkflw_executor::runner_base::published_image(&dockerfile);
    std::fs::create_dir_all(root.join("docker")).unwrap();
    std::fs::write(root.join("docker/runner.Dockerfile"), &dockerfile).unwrap();
    std::fs::write(root.join("docker/runner-image.txt"), format!("{}\n", image)).unwrap();
    println!("{}", image);
}
