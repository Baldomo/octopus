---
name: octopus
description: Drive octopus servers from a shell, covering workspaces, tasks, agent terminals, files, browser panels and pairing. Use when an agent or script creates, moves or inspects octopus tasks, types into their terminals, or opens a page in a task's browser panel.
---

# octopus

Every flag is required. A success prints the op's result as JSON on stdout; a failure prints `error: ` and the reason on stderr and exits non-zero. Inside a task terminal, pass the terminal's own server and task: `--server "$OCTOPUS_SERVER" --task "$OCTOPUS_TASK"`.

## `octopus app`

Open the window onto every server in this client's list.

## `octopus workspace add`

Add a workspace: a directory this server runs tasks in.

- `--server <server>`: The paired server's name.
- `--name <name>`: The workspace's name, unique on its server.
- `--root <root>`: The directory on the server's machine, as an absolute path.
- `--color auto|red-mist|orange-mist|amber-mist|lime-mist|green-mist|mint-mist|teal-mist|sky-mist|blue-mist|indigo-mist|violet-mist|pink-mist|red-soft|orange-soft|amber-soft|lime-soft|green-soft|mint-soft|teal-soft|sky-soft|blue-soft|indigo-soft|violet-soft|pink-soft|red-vivid|orange-vivid|amber-vivid|lime-vivid|green-vivid|mint-vivid|teal-vivid|sky-vivid|blue-vivid|indigo-vivid|violet-vivid|pink-vivid|red-deep|orange-deep|amber-deep|lime-deep|green-deep|mint-deep|teal-deep|sky-deep|blue-deep|indigo-deep|violet-deep|pink-deep`: The workspace's color: auto, or a <hue>-<tone> of the matrix.

## `octopus workspace list`

List the server's workspaces.

- `--server <server>`: The paired server's name.

## `octopus workspace rename`

Rename a workspace; it keeps its root and its tasks.

- `--server <server>`: The paired server's name.
- `--workspace <workspace>`: The workspace, by name.
- `--name <name>`: Its new name, unique on its server.

## `octopus workspace set-color`

Set a workspace's color; it keeps its name, root and tasks.

- `--server <server>`: The paired server's name.
- `--workspace <workspace>`: The workspace, by name.
- `--color auto|red-mist|orange-mist|amber-mist|lime-mist|green-mist|mint-mist|teal-mist|sky-mist|blue-mist|indigo-mist|violet-mist|pink-mist|red-soft|orange-soft|amber-soft|lime-soft|green-soft|mint-soft|teal-soft|sky-soft|blue-soft|indigo-soft|violet-soft|pink-soft|red-vivid|orange-vivid|amber-vivid|lime-vivid|green-vivid|mint-vivid|teal-vivid|sky-vivid|blue-vivid|indigo-vivid|violet-vivid|pink-vivid|red-deep|orange-deep|amber-deep|lime-deep|green-deep|mint-deep|teal-deep|sky-deep|blue-deep|indigo-deep|violet-deep|pink-deep`: Its new color: auto, or a <hue>-<tone> of the matrix.

## `octopus workspace remove`

Remove a workspace with no live task, with every task in it; their worktrees go and their branches stay.

- `--server <server>`: The paired server's name.
- `--workspace <workspace>`: The workspace, by name.

## `octopus harness set`

Declare how a harness starts its agent.

- `--server <server>`: The paired server's name.
- `--id <id>`: The harness's id, which tasks name.
- `--argv <argv>`: The agent's argv as a JSON array of strings, e.g. ["claude"].
- `--start <start>`: The args appended to argv on a session's first start, with {session} for the session id, e.g. ["--session-id","{session}"].
- `--resume <resume>`: The args appended to argv on every later start, with {session} for the session id, e.g. ["--resume","{session}"].
- `--install <install>`: The command that installs the agent, shown when it is missing.
- `--hooks true|false`: Whether the agent reports its status with `agent report`.

## `octopus agent report`

Report what a task's agent is doing; harness hooks call this.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--status working|waiting|idle`: The agent's status.

## `octopus task create`

Create a task.

- `--server <server>`: The paired server's name.
- `--workspace <workspace>`: The workspace the task works in, by name.
- `--title <title>`: The task's title.
- `--parent <parent>`: The parent task's id; none for a root task.
- `--state pinned|temporary|completed|cancelled`: The task's state.
- `--checkout root|worktree`: Where the task's files live: the workspace root, or a worktree of its own.
- `--harness <harness>`: The harness that runs the task's agent.
- `--session <session>`: An existing session for the agent to resume; none for a fresh session.

## `octopus task move`

Move a task to a state.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--state pinned|temporary|completed|cancelled`: The state to move it to.

## `octopus task rename`

Rename a task; a worktree task's branch keeps its name.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--title <title>`: The task's new title.

## `octopus task set-session`

Point a task's agent at an existing session, ending the agent running now; the next focus resumes it.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--session <session>`: The session to resume.

## `octopus task archive`

Archive an ended task, which takes it off the Tasks board until it moves again.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.

## `octopus task focus`

Focus a task, starting its agent if it has none.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.

## `octopus task get`

Print a task.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.

## `octopus term send`

Type text into a terminal panel.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--panel <panel>`: The panel's name or id, e.g. agent.
- `--text <text>`: The text to type.
- `--enter true|false`: Whether Enter follows the text.
- `--take-lock true|false`: Whether to take the panel's write lock from its holder.

## `octopus term take-lock`

Take a terminal panel's write lock from its holder.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--panel <panel>`: The panel's name or id, e.g. agent.

## `octopus term resize`

Resize a terminal panel; only its lock holder may.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--panel <panel>`: The panel's name or id, e.g. agent.
- `--cols <cols>`: Columns.
- `--rows <rows>`: Rows.

## `octopus term attach`

Stream a terminal panel into this terminal; Ctrl-] detaches.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--panel <panel>`: The panel's name or id, e.g. agent.
- `--take-lock true|false`: Whether to take the panel's write lock and size it to this terminal.

## `octopus file list`

List a directory of a task's checkout.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--path <path>`: The directory, relative to the checkout; . for its root.

## `octopus file write`

Create a file in a task's checkout, or replace its content.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--path <path>`: The file, relative to the checkout.
- `--content <content>`: The file's new content, as text.

## `octopus file search`

Find each line of a task's checkout that contains a text.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--pattern <pattern>`: The literal text to find.

## `octopus file read`

Write a file of a task's checkout to stdout, byte for byte.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--path <path>`: The file, relative to the checkout.

## `octopus browser open`

Show a page in one of a task's browser panels, on every client that renders the task.

- `--server <server>`: The paired server's name.
- `--task <task>`: The task's id.
- `--url <url>`: The page, an http or https URL; one on localhost:<port> reaches that port on the server's machine.
- `--panel <panel>`: `new` for a new browser panel, or an existing browser panel's id.

## `octopus pairing offer`

Show a one-time code that pairs one client with this server.

- `--server <server>`: The paired server's name.

## `octopus pairing list`

List the clients this server has paired with.

- `--server <server>`: The paired server's name.

## `octopus pairing forget`

Forget a paired client, which must pair with a code again; its open connections stay until they close.

- `--server <server>`: The paired server's name.
- `--client <client>`: The client's name, as it paired.

## `octopus pairing accept`

Pair with a server on another machine using the code its `pairing offer` showed, and list it under its name.

- `--address <address>`: The server's --quic address as this machine reaches it, host:port.
- `--code <code>`: The one-time code.

## `octopus server run`

Run this machine's server in the foreground.

- `--name <name>`: The server's name, which agents in its terminals see as OCTOPUS_SERVER.
- `--socket <socket>`: A Unix socket path, or a named pipe `\\.\pipe\<name>` on Windows.
- `--store <store>`: The SQLite file the server keeps its state in.
- `--worktrees <worktrees>`: Where task worktrees live: <worktrees>/<workspace>/<task>.
- `--terminal-host <terminal-host>`: Socket of the terminal host, the process that keeps this server's terminals running across restarts; started if nothing answers there.
- `--quic <quic>`: The UDP address QUIC listens on for clients on other machines, e.g. 0.0.0.0:7300.

## `octopus server install`

Run the server at every login as a user service (launchd, systemd --user, or Task Scheduler).

- `--name <name>`: The server's name, which agents in its terminals see as OCTOPUS_SERVER.
- `--socket <socket>`: A Unix socket path, or a named pipe `\\.\pipe\<name>` on Windows.
- `--store <store>`: The SQLite file the server keeps its state in.
- `--worktrees <worktrees>`: Where task worktrees live: <worktrees>/<workspace>/<task>.
- `--terminal-host <terminal-host>`: Socket of the terminal host, the process that keeps this server's terminals running across restarts; started if nothing answers there.
- `--quic <quic>`: The UDP address QUIC listens on for clients on other machines, e.g. 0.0.0.0:7300.
- `--log <log>`: The file the service's output is appended to.

## `octopus server uninstall`

Remove the login service that `install` registered for this --name.

- `--name <name>`: The --name the service was installed with.

## `octopus server add`

Remember a server on this machine's local socket under a name.

- `--name <name>`: The name to call the server by.
- `--socket <socket>`: The server's --socket.

## `octopus server rename`

Call a listed server by another name in this client's list; the server is not told.

- `--server <server>`: The paired server's name.
- `--name <name>`: The name to call the server by.

## `octopus server forget`

Drop a server and its pin from this client's list; the server keeps this client paired.

- `--server <server>`: The paired server's name.
