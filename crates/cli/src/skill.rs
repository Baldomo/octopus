//! The octopus skill: every command and flag of the command tree, rendered for an agent to read.

use clap::Command;

/// Where the rendered skill lives, relative to the workspace root.
pub const PATH: &str = "skills/octopus/SKILL.md";

const HEAD: &str = r#"---
name: octopus
description: Drive octopus servers from a shell, covering workspaces, tasks, agent terminals, files, browser panels and pairing. Use when an agent or script creates, moves or inspects octopus tasks, types into their terminals, or opens a page in a task's browser panel.
---

# octopus

Every flag is required. A success prints the op's result as JSON on stdout; a failure prints `error: ` and the reason on stderr and exits non-zero. Inside a task terminal, pass the terminal's own server and task: `--server "$OCTOPUS_SERVER" --task "$OCTOPUS_TASK"`.
"#;

fn value(arg: &clap::Arg) -> String {
    let choices: Vec<String> = arg.get_possible_values().iter().map(|choice| choice.get_name().to_string()).collect();
    if choices.is_empty() {
        let names = arg.get_value_names().unwrap_or_default();
        names.iter().map(|name| format!("<{}>", name.to_lowercase().replace('_', "-"))).collect::<Vec<_>>().join(" ")
    } else {
        choices.join("|")
    }
}

/// Clap drops the closing period of a derived doc comment.
fn sentence(text: impl ToString) -> String {
    let text = text.to_string();
    if text.ends_with('.') { text } else { format!("{text}.") }
}

fn render_leaf(path: &str, command: &Command, out: &mut String) {
    out.push_str(&format!("\n## `octopus {path}`\n\n"));
    if let Some(about) = command.get_about() {
        out.push_str(&format!("{}\n", sentence(about)));
    }
    let flags: Vec<&clap::Arg> = command.get_arguments().filter(|arg| arg.get_long().is_some()).collect();
    if !flags.is_empty() {
        out.push('\n');
    }
    for arg in flags {
        let long = arg.get_long().expect("filtered to long flags");
        let help = sentence(arg.get_help().expect("every flag is documented"));
        out.push_str(&format!("- `--{long} {}`: {help}\n", value(arg)));
    }
}

fn render_tree(path: &str, command: &Command, out: &mut String) {
    if command.is_hide_set() {
        return;
    }
    if command.has_subcommands() {
        for sub in command.get_subcommands() {
            let sub_path =
                if path.is_empty() { sub.get_name().to_string() } else { format!("{path} {}", sub.get_name()) };
            render_tree(&sub_path, sub, out);
        }
    } else {
        render_leaf(path, command, out);
    }
}

pub fn render(tree: &Command) -> String {
    let mut out = HEAD.to_string();
    render_tree("", tree, &mut out);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_skill_documents_the_current_commands() {
        let committed = include_str!("../../../skills/octopus/SKILL.md");
        assert!(
            committed == super::render(&crate::commands::tree()),
            "{} drifted from the commands; run `cargo run -p octopus-cli --example skill`",
            super::PATH
        );
    }
}
