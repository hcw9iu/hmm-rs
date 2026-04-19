use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearIssue {
    pub identifier: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearIssueDetails {
    pub identifier: String,
    pub url: Option<String>,
    pub state: Option<String>,
    pub project: Option<String>,
}

pub fn detect_team_slug(workdir: &str) -> Result<String, String> {
    if let Ok(team) = std::env::var("HMM_LINEAR_TEAM") {
        let team = team.trim();
        if !team.is_empty() {
            return Ok(team.to_string());
        }
    }

    let output = Command::new("linear")
        .args(["team", "list"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("failed to run linear team list: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("KEY ") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let _key = parts.next();
        if let Some(slug) = parts.next() {
            return Ok(slug.to_string());
        }
    }

    Err("could not detect Linear team slug".to_string())
}

pub fn current_git_head(workdir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workdir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

pub fn create_issue(
    workdir: &str,
    team_slug: &str,
    title: &str,
    parent_identifier: Option<&str>,
) -> Result<LinearIssue, String> {
    let mut args = vec![
        "issue",
        "create",
        "--team",
        team_slug,
        "--title",
        title,
        "--no-interactive",
    ];
    if let Some(parent) = parent_identifier {
        args.extend(["--parent", parent]);
    }

    let output = Command::new("linear")
        .args(args)
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("failed to run linear issue create: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }

    parse_issue_from_output(&String::from_utf8_lossy(&output.stdout))
}

pub fn update_issue(
    workdir: &str,
    identifier: &str,
    title: &str,
    parent_identifier: Option<&str>,
) -> Result<LinearIssue, String> {
    let mut args = vec!["issue", "update", identifier, "--title", title];
    if let Some(parent) = parent_identifier {
        args.extend(["--parent", parent]);
    }

    let output = Command::new("linear")
        .args(args)
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("failed to run linear issue update: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }

    parse_issue_from_output(&String::from_utf8_lossy(&output.stdout))
}

pub fn issue_url(workdir: &str, identifier: &str) -> Result<String, String> {
    let output = Command::new("linear")
        .args(["issue", "url", identifier])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("failed to run linear issue url: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://linear.app/"))
        .map(|s| s.to_string())
        .ok_or_else(|| "could not read Linear issue URL".to_string())
}

pub fn issue_details(workdir: &str, identifier: &str) -> Result<LinearIssueDetails, String> {
    let output = Command::new("linear")
        .args(["issue", "view", identifier, "--json"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("failed to run linear issue view: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_issue_details_json(&json)
}

fn parse_issue_details_json(json: &str) -> Result<LinearIssueDetails, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("failed to parse linear issue json: {}", e))?;

    let identifier = value
        .get("identifier")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing identifier in Linear issue json".to_string())?
        .to_string();

    let url = value
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let state = value
        .get("state")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let project = value
        .get("project")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(LinearIssueDetails {
        identifier,
        url,
        state,
        project,
    })
}

fn parse_issue_from_output(stdout: &str) -> Result<LinearIssue, String> {
    let url = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://linear.app/"))
        .map(|s| s.to_string());

    if let Some(url) = url {
        let identifier = parse_identifier_from_url(&url)
            .ok_or_else(|| format!("could not parse issue identifier from URL: {}", url))?;
        return Ok(LinearIssue {
            identifier,
            url: Some(url),
        });
    }

    for token in stdout.split_whitespace() {
        if looks_like_issue_identifier(token) {
            return Ok(LinearIssue {
                identifier: token.trim_end_matches(':').to_string(),
                url: None,
            });
        }
    }

    Err(format!("could not parse issue output: {}", stdout.trim()))
}

fn parse_identifier_from_url(url: &str) -> Option<String> {
    let issue_marker = "/issue/";
    let idx = url.find(issue_marker)?;
    let rest = &url[idx + issue_marker.len()..];
    rest.split('/').next().map(|s| s.to_string())
}

fn looks_like_issue_identifier(token: &str) -> bool {
    let token = token.trim_end_matches(':');
    let Some((prefix, number)) = token.rsplit_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_create_output_url() {
        let issue = parse_issue_from_output(
            "Creating issue in HCW\n\nhttps://linear.app/hcw/issue/HCW-196/hmm-cli-smoke-test\n",
        )
        .unwrap();
        assert_eq!(issue.identifier, "HCW-196");
    }

    #[test]
    fn parses_update_output() {
        let issue = parse_issue_from_output(
            "Updating issue HCW-196\n\n✓ Updated issue HCW-196: Test\nhttps://linear.app/hcw/issue/HCW-196/test\n",
        )
        .unwrap();
        assert_eq!(issue.identifier, "HCW-196");
    }

    #[test]
    fn parses_issue_details_json() {
        let details = parse_issue_details_json(
            r##"{
  "identifier": "HCW-199",
  "url": "https://linear.app/hcw/issue/HCW-199/hmm-issue-detail-probe",
  "state": { "name": "Backlog", "color": "#bec2c8" },
  "project": null
}"##,
        )
        .unwrap();
        assert_eq!(details.identifier, "HCW-199");
        assert_eq!(details.state.as_deref(), Some("Backlog"));
        assert_eq!(details.project, None);
    }
}
