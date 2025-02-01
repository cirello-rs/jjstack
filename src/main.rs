// Copyright 2024 http://github.com/cirello-io/jjstack U. Cirello
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::json;

const STACK_HEADER: &str = "<!-- STACK NAVIGATION -->";
const STACK_FOOTER: &str = "<!-- END STACK NAVIGATION -->";

#[derive(Debug, Deserialize)]
struct GithubPullRequest {
    #[serde(rename = "number")]
    number: i32,
    #[serde(rename = "title")]
    title: String,
    #[serde(rename = "body")]
    body: Option<String>,
    #[serde(rename = "head")]
    head: GithubReference,
    #[serde(rename = "base")]
    base: GithubReference,
}

#[derive(Debug, Deserialize)]
struct GithubReference {
    #[serde(rename = "ref")]
    r#ref: String,
}

#[derive(Clone)]
struct PullRequest {
    number: i32,
    title: String,
    head: String,
    base: String,
    body: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new("gh")
        .args(["repo", "set-default", "--view"])
        .output()?;
    if !out.status.success() {
        return Err(format!(
            "cannot run 'gh repo set-default --view': {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    let repo = String::from_utf8(out.stdout)?.trim().to_string();
    println!("repo: {:?}", repo);

    let args: Vec<String> = env::args().collect();
    let apply = args.contains(&"--apply".to_string());

    let bookmarks = get_bookmarks()?;
    if bookmarks.is_empty() {
        println!("no bookmarks found.");
        return Ok(());
    }

    let bookmark_idx: HashSet<String> = bookmarks.into_iter().collect();
    let prs = get_open_prs(repo.to_string(), bookmark_idx.clone())?;
    if prs.is_empty() {
        println!("no matching PRs found for bookmarks.");
        return Ok(());
    }

    let pr_stacks = build_pr_stacks(prs);
    for stack in pr_stacks {
        if stack.len() > 1 {
            for pr in &stack {
                let nav_block = generate_nav_block(stack.clone(), pr.head.to_string());
                if apply {
                    if let Err(e) = update_pr_description(pr.clone(), nav_block, repo.to_string()) {
                        eprintln!("#{}: cannot update PR: {}", pr.number, e);
                        continue;
                    }
                    println!("PR #{} {:?}: updated", pr.number, pr.title);
                } else {
                    println!("PR #{} {:?}: updates with", pr.number, pr.title);
                    for line in nav_block.lines() {
                        println!("\t{}", line);
                    }
                    println!();
                }
            }
        } else {
            let pr = &stack[0];
            if !pr.body.contains(STACK_HEADER) && !pr.body.contains(STACK_FOOTER) {
                continue;
            }
            if apply {
                if let Err(e) = update_pr_description(pr.clone(), "".to_string(), repo.to_string())
                {
                    eprintln!(
                        "#{}: cannot remove navigation block from PR: {}",
                        pr.number, e
                    );
                    continue;
                }
            }
            println!("PR #{} {:?}: removed", pr.number, pr.title);
        }
    }
    Ok(())
}

fn get_bookmarks() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let out = Command::new("jj").args(["bookmark", "list"]).output()?;
    if !out.status.success() {
        return Err(format!(
            "cannot run 'jj bookmark list': {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    let text = String::from_utf8(out.stdout)?;
    let mut bookmarks = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((bookmark, _)) = line.split_once(':') {
            bookmarks.push(bookmark.trim().to_string());
        } else {
            eprintln!("skipping malformed bookmark line: {:?}", line);
        }
    }
    Ok(bookmarks)
}

fn get_open_prs(
    repo: String,
    bookmarks_idx: HashSet<String>,
) -> Result<Vec<PullRequest>, Box<dyn std::error::Error>> {
    let url = format!("repos/{}/pulls", repo);
    let out = Command::new("gh").args(["api", &url]).output()?;
    if !out.status.success() {
        return Err(format!(
            "cannot run 'gh api {}': {}",
            url,
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    let gh_prs: Vec<GithubPullRequest> = serde_json::from_slice(&out.stdout)?;
    let mut prs = Vec::new();
    for gh in gh_prs {
        if bookmarks_idx.contains(&gh.head.r#ref) {
            prs.push(PullRequest {
                number: gh.number,
                title: gh.title,
                head: gh.head.r#ref,
                base: gh.base.r#ref,
                body: gh.body.unwrap_or_default(),
            });
        }
    }
    Ok(prs)
}

fn build_pr_stacks(prs: Vec<PullRequest>) -> Vec<Vec<PullRequest>> {
    let mut head: HashMap<String, PullRequest> = HashMap::new();
    for pr in &prs {
        head.insert(pr.head.clone(), pr.clone());
    }
    let mut child_idx: HashMap<String, PullRequest> = HashMap::new();
    for pr in &prs {
        if let Some(parent) = head.get(&pr.base) {
            child_idx.insert(parent.head.clone(), pr.clone());
        }
    }
    let mut visited = HashSet::new();
    let mut stacks = Vec::new();
    for pr in &prs {
        if visited.contains(&pr.head) {
            continue;
        }
        let mut current = pr.clone();
        while let Some(parent) = head.get(&current.base) {
            current = parent.clone();
        }
        let mut chain = Vec::new();
        loop {
            visited.insert(current.head.clone());
            chain.push(current.clone());
            if let Some(next) = child_idx.get(&current.head) {
                current = next.clone();
            } else {
                break;
            }
        }
        stacks.push(chain);
    }
    stacks
}

fn generate_nav_block(chain: Vec<PullRequest>, current_branch: String) -> String {
    let mut s = String::new();
    use std::fmt::Write;
    writeln!(s, "{}", STACK_HEADER).unwrap();
    writeln!(s, "Stack of changes:").unwrap();
    for (i, pr) in chain.iter().enumerate() {
        let suffix = if pr.head == current_branch {
            " ◁"
        } else {
            ""
        };
        writeln!(
            s,
            "{}. PR #{} (branch: {}){}",
            i + 1,
            pr.number,
            pr.head,
            suffix
        )
        .unwrap();
    }
    writeln!(s, "{}", STACK_FOOTER).unwrap();
    s
}

fn update_pr_description(
    pr: PullRequest,
    nav_block: String,
    repo: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("repos/{}/pulls/{}", repo, pr.number);
    let out = Command::new("gh").args(["api", &url]).output()?;
    if !out.status.success() {
        return Err(format!(
            "cannot run 'gh api {}': {}",
            url,
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    let gh_pr: GithubPullRequest = serde_json::from_slice(&out.stdout)?;
    let gh_pr_body = gh_pr.body.unwrap_or("".to_string());
    let mut new_body = remove_nav_block(gh_pr_body.to_string());
    if !nav_block.is_empty() {
        if !new_body.is_empty() && !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        new_body.push('\n');
        new_body.push_str(&nav_block);
        new_body.push('\n');
    }
    if new_body == gh_pr_body {
        return Ok(());
    }
    let patch_data = serde_json::to_string(&json!({ "body": new_body }))?;
    let mut patch_cmd = Command::new("gh")
        .args(["api", "--input", "-", "-X", "PATCH", &url])
        .stdin(Stdio::piped())
        .spawn()?;
    {
        let stdin = patch_cmd.stdin.as_mut().ok_or("failed to open stdin")?;
        stdin.write_all(patch_data.as_bytes())?;
    }
    let out = patch_cmd.wait_with_output()?;
    if !out.status.success() {
        return Err(format!(
            "cannot run 'gh api -X PATCH {}': {}",
            url,
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    Ok(())
}

fn remove_nav_block(body: String) -> String {
    let start = match body.find(STACK_HEADER) {
        Some(pos) => pos,
        None => return body.to_string(),
    };

    let end = match body.find(STACK_FOOTER) {
        Some(pos) => pos + STACK_FOOTER.len(),
        None => return body.to_string(),
    };

    let before = body[..start].trim();
    let after = body[end..].trim();

    if before.is_empty() || after.is_empty() {
        return format!("{}{}", before, after);
    }

    format!("{}\n{}", before, after)
}
