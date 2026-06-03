use anyhow::{Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use tabled::{Table, Tabled};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "git-branch-check")]
#[command(about = "Batch check if Git repo branches contain latest main branch commits")]
struct Cli {
    /// Root directory to scan for Git repositories
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Threshold for commits behind (exit with error if any repo exceeds this)
    #[arg(short, long, default_value_t = 10)]
    threshold: u32,

    /// Skip git fetch
    #[arg(long)]
    no_fetch: bool,
}

#[derive(Tabled)]
struct RepoStatus {
    #[tabled(rename = "Relative Path")]
    relative_path: String,
    #[tabled(rename = "Current Branch")]
    current_branch: String,
    #[tabled(rename = "Behind")]
    behind: u32,
}

fn find_git_repos(root: &Path) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != "node_modules" && name != "vendor" && name != ".cargo"
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_dir() && entry.file_name() == ".git" {
            if let Some(parent) = entry.path().parent() {
                repos.push(parent.to_path_buf());
            }
        }
    }
    repos
}

fn git_command(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .with_context(|| format!("Failed to run git {:?} in {}", args, repo.display()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {:?} failed: {}", args, stderr.trim());
    }
}

fn fetch_repo(repo: &Path) -> Result<()> {
    git_command(repo, &["fetch", "--all", "--quiet"])?;
    Ok(())
}

fn detect_main_branch(repo: &Path) -> Result<String> {
    // Try to get the remote HEAD (most reliable after fetch)
    if let Ok(output) = git_command(repo, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        // Returns something like "refs/remotes/origin/main"
        if let Some(branch) = output.strip_prefix("refs/remotes/origin/") {
            return Ok(branch.to_string());
        }
    }

    // Fallback: check common main branch names on remote
    for candidate in &["main", "master", "develop", "trunk"] {
        let ref_name = format!("refs/remotes/origin/{}", candidate);
        if git_command(repo, &["rev-parse", "--verify", &ref_name]).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    // Last resort: check local branches
    for candidate in &["main", "master"] {
        if git_command(repo, &["rev-parse", "--verify", *candidate]).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("cannot detect main branch")
}

fn get_current_branch(repo: &Path) -> Result<String> {
    git_command(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn get_commits_behind(repo: &Path, current_branch: &str, main_branch: &str) -> Result<u32> {
    // Count commits in remote main that are not in current branch
    let remote_ref = format!("origin/{}", main_branch);
    let range = format!("{}..{}", current_branch, remote_ref);
    let output = git_command(repo, &["rev-list", "--count", &range])?;
    output
        .parse::<u32>()
        .with_context(|| format!("Failed to parse commit count: {}", output))
}

fn check_repo(repo: &Path, root: &Path, no_fetch: bool) -> Result<RepoStatus> {
    let relative_path = repo
        .strip_prefix(root)
        .unwrap_or(repo)
        .to_string_lossy()
        .to_string();
    let relative_path = if relative_path.is_empty() {
        ".".to_string()
    } else {
        relative_path
    };

    if !no_fetch {
        fetch_repo(repo).with_context(|| format!("fetch failed for {}", relative_path))?;
    }

    let main_branch = detect_main_branch(repo)
        .with_context(|| format!("cannot detect main branch for {}", relative_path))?;

    let current_branch = get_current_branch(repo)
        .with_context(|| format!("cannot get current branch for {}", relative_path))?;

    if current_branch == "HEAD" {
        return Ok(RepoStatus {
            relative_path,
            current_branch: "HEAD (detached)".into(),
            behind: 0,
        });
    }

    let behind = if current_branch == main_branch {
        // On main branch, check if local is behind remote
        let range = format!("HEAD..origin/{}", main_branch);
        git_command(repo, &["rev-list", "--count", &range])
            .unwrap_or_else(|_| "0".into())
            .parse::<u32>()
            .unwrap_or(0)
    } else {
        get_commits_behind(repo, &current_branch, &main_branch).unwrap_or(0)
    };

    Ok(RepoStatus {
        relative_path,
        current_branch,
        behind,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let root = if cli.path.is_absolute() {
        cli.path.clone()
    } else {
        std::env::current_dir()
            .expect("Failed to get current directory")
            .join(&cli.path)
    };

    if !root.exists() {
        eprintln!("Error: path '{}' does not exist", root.display());
        return ExitCode::from(2);
    }

    eprintln!("Scanning for Git repositories in: {}", root.display());

    let repos = find_git_repos(&root);

    if repos.is_empty() {
        eprintln!("No Git repositories found.");
        return ExitCode::SUCCESS;
    }

    eprintln!("Found {} repositories. Checking branches...\n", repos.len());

    let mut results: Vec<RepoStatus> = Vec::new();
    let mut exceeded = false;

    for repo in &repos {
        match check_repo(repo, &root, cli.no_fetch) {
            Ok(status) => {
                if status.behind > cli.threshold {
                    exceeded = true;
                }
                results.push(status);
            }
            Err(e) => {
                eprintln!("WARN: skipping {}: {:#}", repo.display(), e);
            }
        }
    }

    let table = Table::new(&results).to_string();
    println!("{}", table);
    println!();

    if exceeded {
        let bad: Vec<&RepoStatus> = results
            .iter()
            .filter(|r| r.behind > cli.threshold)
            .collect();
        eprintln!(
            "ERROR: {} repo(s) behind by more than {} commits:",
            bad.len(),
            cli.threshold
        );
        for r in &bad {
            eprintln!("  {} [{}] behind {}", r.relative_path, r.current_branch, r.behind);
        }
        return ExitCode::from(1);
    }

    eprintln!("OK: all repos within threshold ({} commits).", cli.threshold);
    ExitCode::SUCCESS
}
