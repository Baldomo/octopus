<img src="docs/readme/banner.svg" alt="octopus: collaborative PEER-TO-PEER control surface for your agents. Across every machine you own." width="100%">

<p align="center">
  <a href="#quick-start">Quick start</a>
  ·
  <a href="docs/CONTRIBUTING.md">Contributing</a>
  ·
  <a href="docs/support.md">Support</a>
  ·
  <a href="docs/privacy.md">Privacy</a>
</p>

<img src="docs/readme/hero.webp" alt="octopus on the laptop: the sidebar with every workspace and its tasks, the task's browser on Supabase Studio, and the agent's terminal writing the migration it shows" width="100%">

### One terminal, many screens

Open the same task on your laptop and your phone. A visible lock shows which device types into the terminal, and any other takes over with one tap.

<img src="docs/readme/lock.webp" alt="The same task on the laptop, read-only and held by the iPhone, and on the phone, which types into its terminal" width="100%">

### Agents in parallel

Every task runs its own Claude Code agent in its own terminal. Tasks nest into subtasks and group by workspace, and a glance at the sidebar or the Tasks board shows which agents are working and which wait on you.

<img src="docs/readme/parallel.webp" alt="An agent's terminal on the laptop, the phone's list of every computer's tasks, and an agent waiting on a permission prompt" width="100%">

### Files, browser and terminals in one window

Each task has an editor, a browser and terminal panels beside its agent.

<img src="docs/readme/window.webp" alt="The laptop with the task's browser on its pull request beside the agent, and the same browser on the phone" width="100%">

### Pair a server, share it

Pair a device by scanning a server's QR code or pasting its invite. Hand the same invite to a teammate and their account gets in, every device of theirs with it. The server's menu lists each account, and any of them can be revoked.

<img src="docs/readme/share.webp" alt="The laptop showing a server's one-time pairing code, and a teammate's phone that read it, about to pair" width="100%">

### And

- **Any folder, three ways.** A task works in any folder on its server: in place, in a git worktree of its own, or in a worktree that also clones your ignored files such as `.env` and `node_modules` (on a file system that clones files, like APFS).
- **A phone app for iOS and Android.** Pair it with your servers by scanning a QR code, then check on agents and answer them from anywhere.
- **Your machines.** Tasks run on your laptop, and on any Mac mini or Linux box you install the server on. Other devices reach them through an iroh relay that forwards packets it cannot read: n0's public one or your own.

## Download

The [Releases page](https://github.com/zabealbe/octopus/releases) has the app for macOS (universal `.dmg`) and Linux (amd64 and arm64 `.AppImage`), the Android app (`.apk`), and the headless CLI and server for Linux (x86_64 and aarch64 `.tar.gz`). On a Mac, the app carries the same `octopus` command, at `octopus.app/Contents/MacOS/octopus`.

Verify a download against the build that made it, then check it against the release's `SHA256SUMS`, which sits beside it in the same folder:

```sh
gh attestation verify octopus_<version>_universal.dmg --repo zabealbe/octopus
shasum -a 256 --check --ignore-missing SHA256SUMS
```

## Quick start

Download the app from the Releases page and open it. On macOS it offers to move itself to Applications first; on Linux, mark the AppImage executable first (`chmod +x octopus_*.AppImage`).

Every start of the app checks this machine's server and repairs it: registered as a login service, running the installed binary, and answering with at least this version.

On a machine without a display, like a Linux box, unpack the headless CLI from the Releases page, install its server, and offer to pair:

```sh
tar -xzf octopus-cli_<version>_<target>.tar.gz
./octopus install
octopus pairing offer --server "$(hostname)"
```

`octopus install` copies the binary to `~/.octopus/bin/octopus`, links `~/.local/bin/octopus` to it, and registers its server with systemd. Every later run of the CLI repairs that server the way the app does, so running a newer release's `octopus` once upgrades it.

Paste the invite into **Pair a server** (the `+` after the server names in the sidebar), or scan its QR code with the phone app. The iOS app builds from source.

Developers start at [CONTRIBUTING.md](docs/CONTRIBUTING.md).
