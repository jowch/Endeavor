# Test hosts for remote sessions

Development only; nothing here ships.

- **Linux server and Slurm cluster:** an [OrbStack](https://orbstack.dev) Linux machine.
  1. `orbctl create ubuntu endeavor-linux`
  2. Add an SSH alias so the app can use it like any server:
     ```
     Host endeavor-linux
       HostName 127.0.0.1
       Port 32222
       User endeavor-linux
       IdentityFile ~/.orbstack/ssh/id_ed25519
       StrictHostKeyChecking accept-new
     ```
  3. `scripts/test-hosts/slurm-setup.sh endeavor-linux` makes it a one-node
     Slurm cluster (partitions `shared`, 8 h default, and `short`, 1 h). Safe to
     re-run.
  4. `scripts/build-helpers.sh --via endeavor-linux` builds the Linux helper
     inside it. Rebuild after changing `crates/`, or the app installs a stale
     helper on the server.
- **The app itself on Linux:** the same machine can build and run the app
  under Xvfb; the packages and steps are in [docs/linux.md](../../docs/linux.md).
- **This Mac as a server:** turn on Remote Login and add your public key to
  `~/.ssh/authorized_keys`, so `ssh localhost` works without a prompt.

There's no x86_64 Linux test host yet.
