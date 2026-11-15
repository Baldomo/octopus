<h1 align="center">
  <img src="crates/desktop/icons/icon.png" alt="" width="128">
  <br>octopus
</h1>

<p align="center">
  Run your coding agents in parallel, on your own machines, from your laptop or your phone.
  <br>
  <a href="#quick-start">Quick start</a>
  ·
  <a href="docs/CONTRIBUTING.md">Contributing</a>
</p>

<img src="https://github.com/zabealbe/octopus/raw/assets/readme/hero.png" alt="One task on three devices: the tablet holds the agent's terminal and fills its screen, the laptop shows the same terminal read-only beside a k6 load-test report in its browser with Take over, and the phone lists the live tasks with each agent's status" width="100%">

- **Agents in parallel.** Every task runs its own Claude Code agent in its own terminal. Tasks nest into subtasks and group by workspace, and a glance at the sidebar or the Tasks board shows which agents are working and which wait on you.
- **Any folder, three ways.** A task works in any folder on its server: in place, in a git worktree of its own, or in a worktree that also clones your ignored files such as `.env` and `node_modules` (on a file system that clones files, like APFS).
- **Files, browser and terminals in one window.** Each task has an editor, a browser and terminal panels beside its agent.
- **One terminal, many screens.** Open the same task on your laptop and your phone. A visible lock shows which device types into the terminal, and any other takes over with one tap.
- **A phone app for iOS and Android.** Pair it with your servers by scanning a QR code, then check on agents and answer them from anywhere.
- **Your machines.** Tasks run on your laptop, and on any Mac mini or Linux box you install the server on. Other devices reach them through an iroh relay that forwards packets it cannot read: n0's public one or your own.

<img src="https://github.com/zabealbe/octopus/raw/assets/readme/board.png" alt="The Tasks board: a workspace's tasks in pinned, temporary, completed and cancelled columns, each live task marked working, needs you or idle" width="100%">

<img src="https://github.com/zabealbe/octopus/raw/assets/readme/devices.png" alt="The phone app listing tasks, and following an agent mid-run" width="100%">

## Quick start

Install the app, the CLI and a server on your laptop (macOS or Linux, with Rust and Node):

```sh
npm --prefix ui ci && npm --prefix ui run build
cargo install --locked --path crates/cli
octopus app
```

On Linux, first install `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libasound2-dev`.

The first start registers this machine's server as a login service.

On a machine without a display, like a Mac mini, clone the repository and add it as a second server:

```sh
cargo install --locked --path crates/cli --no-default-features
octopus server install --name mac-mini \
  --socket ~/.octopus/server.sock --store ~/.octopus/store.db \
  --worktrees ~/.octopus/worktrees --terminal-host ~/.octopus/host.sock \
  --udp 0.0.0.0:7300 --relay https://euc1-1.relay.n0.iroh.link./ \
  --microphone false --log ~/.octopus/server.log
octopus server add --name mac-mini --socket ~/.octopus/server.sock
octopus pairing offer --server mac-mini
```

Paste the invite into **Pair a server** (the `+` after the server names in the sidebar), or scan its QR code with the phone app. The phone app builds from source.

Developers start at [CONTRIBUTING.md](docs/CONTRIBUTING.md).
