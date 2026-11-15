//! Local organization: projects (nested folders), filing rules, and the
//! suggestions that make first-time setup one click.

use crate::model::{AgentRecord, Project, Rule, RuleField, Store, Suggestion};
use std::collections::BTreeMap;

/// Case-insensitive glob with `*` wildcards.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn field_values(agent: &AgentRecord, field: RuleField) -> Vec<String> {
    let c = &agent.card;
    let one = |v: &Option<String>| v.iter().cloned().collect::<Vec<_>>();
    match field {
        RuleField::Address => vec![agent.address.clone()],
        RuleField::Repo => one(&c.repo),
        RuleField::Host => one(&c.host),
        RuleField::Harness => one(&c.harness),
        RuleField::Branch => one(&c.branch),
        RuleField::Tag => c.tags.clone(),
        RuleField::Hint => one(&c.project_hint),
        RuleField::Name => one(&c.name),
    }
}

pub fn rule_matches(rule: &Rule, agent: &AgentRecord) -> bool {
    rule.enabled && field_values(agent, rule.field).iter().any(|v| glob_match(&rule.pattern, v))
}

pub fn project_exists(projects: &[Project], id: &str) -> bool {
    projects.iter().any(|p| p.id == id)
}

/// Names from the root down to `id`.
pub fn project_path(projects: &[Project], id: &str) -> Vec<String> {
    let mut path = Vec::new();
    let mut cursor = Some(id.to_string());
    let mut guard = 0;
    while let Some(current) = cursor {
        guard += 1;
        if guard > 32 {
            break;
        }
        match projects.iter().find(|p| p.id == current) {
            Some(p) => {
                path.push(p.name.clone());
                cursor = p.parent_id.clone();
            }
            None => break,
        }
    }
    path.reverse();
    path
}

pub fn find_by_path(projects: &[Project], path: &[String]) -> Option<String> {
    let mut parent: Option<String> = None;
    for name in path {
        let found = projects
            .iter()
            .find(|p| p.parent_id == parent && p.name.eq_ignore_ascii_case(name))?;
        parent = Some(found.id.clone());
    }
    parent
}

/// Create any missing folders along `path`; returns the leaf id.
pub fn ensure_path(projects: &mut Vec<Project>, path: &[String]) -> Option<String> {
    let mut parent: Option<String> = None;
    for name in path {
        let existing = projects
            .iter()
            .find(|p| p.parent_id == parent && p.name.eq_ignore_ascii_case(name))
            .map(|p| p.id.clone());
        let id = match existing {
            Some(id) => id,
            None => {
                let id = format!("p-{}", uuid::Uuid::new_v4().simple());
                let order = projects.iter().map(|p| p.order).max().unwrap_or(0) + 1;
                projects.push(Project {
                    id: id.clone(),
                    name: name.clone(),
                    parent_id: parent.clone(),
                    color: None,
                    notify: Default::default(),
                    order,
                });
                id
            }
        };
        parent = Some(id);
    }
    parent
}

/// Ancestors first, then the project itself.
pub fn lineage(projects: &[Project], id: &str) -> Vec<Project> {
    let mut chain = Vec::new();
    let mut cursor = Some(id.to_string());
    while let Some(current) = cursor {
        if chain.len() > 32 {
            break;
        }
        match projects.iter().find(|p| p.id == current) {
            Some(p) => {
                chain.push(p.clone());
                cursor = p.parent_id.clone();
            }
            None => break,
        }
    }
    chain.reverse();
    chain
}

/// Resolve where an agent is filed: explicit agent filing, then the first
/// matching rule, then a self-reported hint that names an existing project.
pub fn resolve_agent(store: &Store, agent: &AgentRecord) -> (Option<String>, String) {
    if let Some(id) = &agent.project_id {
        if project_exists(&store.projects, id) {
            return (Some(id.clone()), "agent".into());
        }
    }
    let hinted = if store.settings.use_project_hints {
        agent.card.project_hint.as_ref().and_then(|hint| {
            let path: Vec<String> =
                hint.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if path.is_empty() {
                None
            } else {
                find_by_path(&store.projects, &path)
            }
        })
    } else {
        None
    };
    for rule in &store.rules {
        if project_exists(&store.projects, &rule.project_id) && rule_matches(rule, agent) {
            // A hint may refine a rule into one of that project's own subfolders,
            // but never move the agent somewhere the rule did not put it.
            if let Some(id) = &hinted {
                if id != &rule.project_id && lineage(&store.projects, id).iter().any(|p| p.id == rule.project_id) {
                    return (Some(id.clone()), "hint".into());
                }
            }
            return (Some(rule.project_id.clone()), format!("rule:{}", rule.id));
        }
    }
    if let Some(id) = hinted {
        return (Some(id), "hint".into());
    }
    (None, "none".into())
}

fn namespace_of(address: &str) -> Option<(String, String)> {
    let cut = match address.rfind('/') {
        Some(slash) if slash > 0 && !address[..slash].ends_with('/') && !address[..slash].ends_with(':') => slash,
        _ => address.rfind(':')?,
    };
    if cut == 0 || cut + 1 >= address.len() {
        return None;
    }
    let prefix = &address[..=cut];
    let trimmed = prefix.trim_end_matches(['/', ':']);
    let name = trimmed.rsplit(['/', ':']).find(|s| !s.is_empty())?.to_string();
    if name.is_empty() || name.eq_ignore_ascii_case("telex") && trimmed.ends_with("//") {
        return None;
    }
    Some((prefix.to_string(), name))
}

/// One-click setup suggestions for agents that are not filed anywhere yet.
pub fn suggestions(store: &Store) -> Vec<Suggestion> {
    let unsorted: Vec<&AgentRecord> =
        store.agents.values().filter(|a| resolve_agent(store, a).0.is_none()).collect();
    let mut out: Vec<Suggestion> = Vec::new();
    let mut push = |basis: &str,
                    key: String,
                    title: String,
                    path: Vec<String>,
                    field: RuleField,
                    pattern: String,
                    agents: Vec<String>| {
        let id = format!("{basis}:{key}");
        if agents.is_empty() || store.suggestions_dismissed.contains(&id) {
            return;
        }
        out.push(Suggestion { id, basis: basis.into(), title, project_path: path, field, pattern, agents });
    };

    let mut by_hint: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_ns: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut by_host: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // A hint for a folder that does not exist yet is worth offering even when a
    // broader rule already filed the agent: it is how subfolders get discovered.
    for a in store.agents.values().filter(|a| a.project_id.is_none()) {
        if let Some(h) = &a.card.project_hint {
            let path: Vec<String> =
                h.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if !path.is_empty() && find_by_path(&store.projects, &path).is_none() {
                by_hint.entry(h.clone()).or_default().push(a.address.clone());
            }
        }
    }
    for a in &unsorted {
        if let Some(r) = &a.card.repo {
            by_repo.entry(r.clone()).or_default().push(a.address.clone());
        }
        if let Some(ns) = namespace_of(&a.address) {
            by_ns.entry(ns).or_default().push(a.address.clone());
        }
        if let Some(h) = &a.card.host {
            by_host.entry(h.clone()).or_default().push(a.address.clone());
        }
    }
    for (hint, agents) in by_hint {
        let path: Vec<String> =
            hint.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if path.is_empty() {
            continue;
        }
        push(
            "hint",
            hint.clone(),
            format!("Agents asking to be filed under “{}”", path.join(" › ")),
            path,
            RuleField::Hint,
            hint,
            agents,
        );
    }
    for (repo, agents) in by_repo {
        let name = repo.rsplit('/').next().unwrap_or(&repo).to_string();
        push(
            "repo",
            repo.clone(),
            format!("Agents working in {repo}"),
            vec![name],
            RuleField::Repo,
            repo,
            agents,
        );
    }
    for ((prefix, name), agents) in by_ns {
        if agents.len() < 2 {
            continue;
        }
        push(
            "namespace",
            prefix.clone(),
            format!("Addresses starting with {prefix}"),
            vec![name],
            RuleField::Address,
            format!("{prefix}*"),
            agents,
        );
    }
    for (host, agents) in by_host {
        if agents.len() < 2 {
            continue;
        }
        push(
            "host",
            host.clone(),
            format!("Agents on {host}"),
            vec!["Machines".into(), host.clone()],
            RuleField::Host,
            host,
            agents,
        );
    }
    // Most meaningful basis first; drop a grouping that repeats an earlier one's agents.
    let rank = |basis: &str| match basis {
        "repo" => 0,
        "hint" => 1,
        "namespace" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| {
        rank(&a.basis).cmp(&rank(&b.basis)).then(b.agents.len().cmp(&a.agents.len())).then(a.id.cmp(&b.id))
    });
    let mut seen: Vec<Vec<String>> = Vec::new();
    out.retain(|s| {
        let mut key = s.agents.clone();
        key.sort();
        // Only the address-prefix guess is redundant when it repeats a repo grouping;
        // "by machine" is a different way to organize and stays on offer.
        if s.basis == "namespace" && seen.contains(&key) {
            return false;
        }
        if s.basis == "repo" {
            seen.push(key);
        }
        true
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Card;

    fn agent(address: &str, repo: Option<&str>, hint: Option<&str>) -> AgentRecord {
        AgentRecord {
            address: address.into(),
            card: Card {
                repo: repo.map(Into::into),
                project_hint: hint.map(Into::into),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn glob_is_case_insensitive_with_wildcards() {
        assert!(glob_match("telex://lossyrob/telex/*", "telex://LossyRob/telex/T-A:impl"));
        assert!(glob_match("*impl*", "agent:impl-134"));
        assert!(glob_match("devbox-?", "devbox-?"));
        assert!(!glob_match("agent:review*", "agent:impl-1"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("a", ""));
    }

    #[test]
    fn precedence_is_agent_then_rule_then_hint() {
        let mut store = Store::default();
        let telex = ensure_path(&mut store.projects, &["telex".into()]).unwrap();
        let station = ensure_path(&mut store.projects, &["telex".into(), "station".into()]).unwrap();
        let paw = ensure_path(&mut store.projects, &["paw".into()]).unwrap();
        assert_eq!(project_path(&store.projects, &station), vec!["telex", "station"]);

        let mut a = agent("agent:impl-1", Some("lossyrob/telex"), Some("telex/station"));
        assert_eq!(resolve_agent(&store, &a), (Some(station.clone()), "hint".into()));

        store.rules.push(Rule {
            id: "r1".into(),
            field: RuleField::Repo,
            pattern: "lossyrob/telex".into(),
            project_id: telex.clone(),
            enabled: true,
        });
        // The hint names a subfolder of the rule's project, so it refines the rule.
        assert_eq!(resolve_agent(&store, &a), (Some(station.clone()), "hint".into()));
        // A hint pointing elsewhere never overrides a rule.
        a.card.project_hint = Some("paw".into());
        assert_eq!(resolve_agent(&store, &a), (Some(telex.clone()), "rule:r1".into()));
        a.card.project_hint = None;
        assert_eq!(resolve_agent(&store, &a), (Some(telex), "rule:r1".into()));

        a.project_id = Some(paw.clone());
        assert_eq!(resolve_agent(&store, &a), (Some(paw), "agent".into()));

        a.project_id = Some("deleted".into());
        assert_eq!(resolve_agent(&store, &a).1, "rule:r1");
    }

    #[test]
    fn suggests_groupings_for_unsorted_agents_only() {
        let mut store = Store::default();
        for (addr, repo) in [
            ("telex://lossyrob/telex/impl-1", "lossyrob/telex"),
            ("telex://lossyrob/telex/review-1", "lossyrob/telex"),
            ("agent:solo", "lossyrob/paw"),
        ] {
            store.agents.insert(addr.into(), agent(addr, Some(repo), None));
        }
        let s = suggestions(&store);
        let repo = s.iter().find(|s| s.id == "repo:lossyrob/telex").unwrap();
        assert_eq!(repo.agents.len(), 2);
        assert_eq!(repo.project_path, vec!["telex"]);
        // The address-prefix grouping repeats the repo grouping, so it is folded away.
        assert!(!s.iter().any(|s| s.basis == "namespace"));
        assert_eq!(namespace_of("telex://lossyrob/telex/T-A:impl").unwrap().0, "telex://lossyrob/telex/");
        assert_eq!(namespace_of("agent:impl-1").unwrap(), ("agent:".to_string(), "agent".to_string()));
        assert!(namespace_of("solo").is_none());

        // Filing them removes the suggestion.
        let id = ensure_path(&mut store.projects, &["telex".into()]).unwrap();
        store.rules.push(Rule {
            id: "r".into(),
            field: RuleField::Repo,
            pattern: "lossyrob/telex".into(),
            project_id: id,
            enabled: true,
        });
        assert!(!suggestions(&store).iter().any(|s| s.id == "repo:lossyrob/telex"));
    }
}
