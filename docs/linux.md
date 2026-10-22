# Running cos on a Linux server (EC2)

This guide sets up the cos daemon on an EC2 instance and drives it from the Mac app over Tailscale. Any Linux box with systemd works the same way. The daemon runs Claude Code agents in git worktrees on the server, so your Mac can sleep while they work.

## 1. The instance

- **Architecture.** Either works: arm64 (Graviton, such as `t4g` or `m7g`) or x86_64. cos ships static binaries for both.
- **OS.** Amazon Linux 2023, Ubuntu or Debian. Anything with systemd will do.
- **Size.** Every agent is its own Claude Code process, using a few hundred MB. Pick 4 GB of RAM or more if you'll run several at once. Leave disk space for your repos and one worktree per task.
- **Security group.** You need **no inbound rule** for cos. Tailscale makes outbound connections and does NAT traversal, so port 7700 is never opened to the internet. Keep SSH open until Tailscale works. With `tailscale up --ssh` you can close it afterwards.

## 2. Tailscale, logged in as you

Install it and log in as **your own Tailscale user**:

```sh
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up            # opens a login URL; sign in as yourself
# optional: sudo tailscale up --ssh   (Tailscale SSH, so you can close port 22)
```

**Don't use a tagged auth key** (`--auth-key tskey-…` with tags, as server guides often suggest). cos only lets in devices owned by the same Tailscale user as the host. A tagged machine is owned by its tags, not by a user, so it has no owner to compare against. On a tagged host, `cos remote` refuses to listen and says why. If the box is already tagged, run `sudo tailscale logout`, then `sudo tailscale up` without a key.

## 3. git and Claude Code

```sh
sudo dnf install -y git        # Amazon Linux;  Debian/Ubuntu: sudo apt-get install -y git
curl -fsSL https://claude.ai/install.sh | bash      # Claude Code's native installer, into ~/.local/bin
```

Then log in on the headless box. Pick one:

- **Your Claude subscription:** run `claude` and follow the login flow. It prints a URL. Open it on your Mac and paste the code back into the SSH session. Approve the folder trust prompt for your repos while you're there.
- **An API key:** put it in the cos home, where the daemon's systemd unit reads it. Never put it in the unit itself:

  ```sh
  mkdir -p ~/.cos && umask 077
  echo 'ANTHROPIC_API_KEY=sk-ant-…' >> ~/.cos/daemon.env
  ```

## 4. Install cos

```sh
curl -fsSL https://raw.githubusercontent.com/raykibul/cos/main/install.sh | sh
```

This puts the right static build for the box (`x86_64` or `aarch64`) in `~/.local/bin/cos` and checks it against the release's `SHA256SUMS`. If `~/.local/bin` isn't on your `PATH`, it says so. The Amazon Linux and Debian profiles add it once the folder exists, so log out and back in. `npx cos-agent@latest` does the same where Node is installed. Run either again later to upgrade, then restart the service as it tells you (`systemctl --user restart cos.service`).

To build it yourself, run `make linux` on any machine with Docker (the Mac included). It writes `dist/cos-<version>-<target>.tar.gz`. Copy the file over and run `tar -xzf cos-*.tar.gz -C ~/.local/bin`.

## 5. Your repos

```sh
git clone git@github.com:you/your-repo.git ~/src/your-repo
cos daemon &                   # or skip ahead to step 6 and come back
cos project add ~/src/your-repo
```

Workers push nothing. cos merges into a workspace branch in the clone. `cos workspace new` fetches `origin` first by default, so the clone needs read access to its remote, such as an SSH deploy key. `--from head` starts from the clone's HEAD without fetching.

## 6. The daemon as a service

```sh
cos daemon install             # turns lingering on, writes ~/.config/systemd/user/cos.service, enables and starts it
cos status
```

- **Lingering.** Without it, systemd stops your user's services when your last SSH session ends, so the daemon would stop when you log out. With it on, the daemon keeps running and starts at boot. `cos daemon install` turns it on for you when the system allows that without a password. If not, it says so: run `sudo loginctl enable-linger $USER`, then `cos daemon install` again.
- **Shells without a login session.** AWS Session Manager, `su` and `sudo -u` shells have no `XDG_RUNTIME_DIR`. `install` finds your user's systemd anyway once lingering is on.
- **Settings it records.** The unit records your `PATH`, so the daemon finds `claude`, `git` and `tailscale`. It also records `COS_HOME`, `CLAUDE_CONFIG_DIR`, `COS_RUN_MODEL` and the other `COS_*` settings, if they're set. Run `cos daemon install` again after changing any of them.
- **Logs.** They go to `~/.cos/logs/daemon.log`. `systemctl --user status cos` shows the service itself.
- **Removing it.** `cos daemon uninstall` stops the daemon and removes the unit.

## 7. Remote access

Remote access is on by default, so the daemon listens on Tailscale as soon as it starts. Check it:

```sh
cos remote status              # "listening on 100.x.y.z:7700, …" and the tailnet owner devices must match
```

`cos remote off` turns it off, and `cos remote on` back on.

If it isn't listening, `cos remote status` says why: Tailscale not installed or not running, logged out, or a tagged machine. The daemon tries again every 30 s.

## 8. Connect the Mac

1. In the Mac app, choose **Add a Mac…**. It lists the devices on your tailnet and marks the ones running cos, so the EC2 box shows up there. Pick it, or type its MagicDNS name, such as `ip-172-31-5-9`, or its `100.x` address.
2. The app's first request registers the Mac as a pending device, and the app shows the command to approve it. On the server, run:

   ```sh
   cos devices                  # the Mac is listed as pending
   cos devices approve 1        # by number, name or node ID
   ```

   The app connects as soon as you approve, without a restart.

From the Mac's terminal you can also run `cos --host ip-172-31-5-9 status`. Device and listener management (`cos devices`, `cos remote`) work only on the server itself.

## What's different on Linux

- **`cos sessions`** lists Claude sessions on the server. It needs `ps` (procps) to tell a live session from a dead one whose pid was reused. Without `ps`, it falls back to `/proc` and skips that check.
- **macOS firewall check.** The check behind `tailnet.reason` is macOS-only. On Linux, Tailscale accepts its own traffic on `tailscale0`. If you added a host firewall (ufw, firewalld), allow TCP 7700 on `tailscale0`.
- **Tailscale CLI lookup.** cos looks for `tailscale` on `PATH`, then in `/usr/bin` and `/usr/local/bin`. Set `COS_TAILSCALE` if it's elsewhere.
- **Claude lookup.** cos looks for `claude` on `PATH`, then where its installers put it: `~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin` and nvm's newest Node. Set `COS_CLAUDE_BIN` if it's elsewhere.
- **No systemd.** In a container, for example, `cos daemon install` refuses. Run `cos daemon` under your own supervisor.
