//! Plugin-content consistency that needs no engine: the workflow scripts name only agents the plugin ships.
//!
//! This test lived in the engine's task-handler tests, which read the plugin tree through
//! `local_app_builder_plugin::root()`. It reads only files, so it belongs with the tree.

use std::collections::HashSet;

/// r2-tests-honesty-005: nothing ties the `agentType: '<x>'` literals in the
/// Local App plugin's workflow scripts to the shipped agent roster
/// (`local-app-builder/agents/*.md`) -- neither `check-phase2-plugin.py`
/// (which only checks the roster directory listing, never opens a workflow
/// script) nor any Rust test. A workflow renamed to an `agentType` with no
/// `.md` on disk passed every existing gate. This test closes the Rust half:
/// it reads the live plugin workflow scripts and roster off disk (no
/// hand-typed name list to rot) and fails if extraction comes back empty
/// (fail-closed, matching the still-missing Python gate's intended posture)
/// or if any extracted `agentType` has no matching roster file.
#[test]
fn plugin_workflow_agent_types_match_shipped_agent_roster() {
    let plugin_root = local_app_builder_plugin::root().to_path_buf();
    let workflows_dir = plugin_root.join("workflows");
    let agents_dir = plugin_root.join("agents");

    let roster: HashSet<String> = std::fs::read_dir(&agents_dir)
        .unwrap_or_else(|error| panic!("cannot read agent roster dir {agents_dir:?}: {error}"))
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect();
    assert!(
        !roster.is_empty(),
        "agent roster dir {agents_dir:?} yielded zero .md files; needle extraction cannot be trusted"
    );

    let mut found_agent_types: HashSet<String> = HashSet::new();
    let workflow_scripts = std::fs::read_dir(&workflows_dir)
        .unwrap_or_else(|error| panic!("cannot read workflows dir {workflows_dir:?}: {error}"));
    for entry in workflow_scripts.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("js") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read workflow script {path:?}: {error}"));
        let mut rest = source.as_str();
        while let Some(at) = rest.find("agentType:") {
            rest = &rest[at + "agentType:".len()..];
            let rest_trimmed = rest.trim_start();
            let quote = rest_trimmed
                .chars()
                .next()
                .filter(|char_| *char_ == '\'' || *char_ == '"');
            if let Some(quote) = quote {
                let after_quote = &rest_trimmed[1..];
                if let Some(end) = after_quote.find(quote) {
                    found_agent_types.insert(after_quote[..end].to_string());
                }
            }
            rest = rest_trimmed;
        }
    }
    assert!(
        !found_agent_types.is_empty(),
        "extracted zero `agentType:` literals from {workflows_dir:?}; the needle \
         derivation itself is broken (verify against a known sample before trusting a \
         zero-hit scan of any workflow script)"
    );

    let unrostered: Vec<&String> = found_agent_types
        .iter()
        .filter(|agent_type| !roster.contains(*agent_type))
        .collect();
    assert!(
        unrostered.is_empty(),
        "workflow scripts under {workflows_dir:?} reference agentType(s) {unrostered:?} \
         with no matching {agents_dir:?}/<name>.md in the shipped roster \
         (roster: {roster:?})"
    );
}
