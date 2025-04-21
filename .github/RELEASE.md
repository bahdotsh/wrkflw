# Release Process

This document outlines the steps for creating a new release of wrkflw.

## Automatic Release Process

The project uses a GitHub Actions workflow to automate the release process. Here's how it works:

1. Tag a new version with Git: 
   ```bash
   git tag -a v0.x.y -m "Release v0.x.y"
   ```

2. Push the tag to GitHub:
   ```bash
   git push origin v0.x.y
   ```

3. The GitHub Actions workflow will automatically:
   - Build release binaries for multiple platforms (Linux, macOS, Windows)
   - Generate a changelog using git-cliff
   - Create a GitHub release with the changelog and binaries
   - Upload the release artifacts

## Commit Message Format

To ensure proper changelog generation, please follow the conventional commit format for your commit messages:

- `feat: add new feature` - for new features
- `fix: resolve issue` - for bug fixes
- `docs: update documentation` - for documentation updates
- `style: format code` - for code style changes (no functional changes)
- `refactor: improve code structure` - for code refactoring
- `perf: improve performance` - for performance improvements
- `test: add or update tests` - for test updates
- `chore: update dependencies` - for maintenance tasks

The changelog will be organized based on these commit types.

## Manual Release Steps (if needed)

If you need to create a release manually:

1. Build the release binaries:
   ```bash
   cargo build --release
   ```

2. Generate a changelog:
   ```bash
   git cliff --latest > CHANGELOG.md
   ```

3. Create a new release on GitHub manually and upload the binaries.

## Configuration

- `cliff.toml` - Configuration for git-cliff to generate changelogs
- `.github/workflows/release.yml` - GitHub Actions workflow for releases 