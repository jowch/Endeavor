#!/usr/bin/env bash
# Turn an OrbStack Linux machine into a one-node Slurm test cluster.
# Run from macOS. Safe to re-run: every step checks or overwrites to the same end state.
#
#   scripts/test-hosts/slurm-setup.sh [machine] [user]
#
# Result: munge, mariadb, slurmdbd, slurmctld and slurmd running under systemd,
# partitions `shared` (default, 8h) and `short` (1h), sacct backed by slurmdbd,
# and passwordless `ssh <node>` from the machine to itself for the user.
#
# OrbStack notes:
# - The machine runs systemd, so daemons are ordinary enabled systemd services.
# - cgroup v2 is delegated into the machine, so proctrack/cgroup and task/cgroup work:
#   jobs get their own cgroup with memory and core limits enforced.
# - OrbStack's own `ssh <machine>` sessions are pinned to one CPU fewer than the machine
#   has (nproc shows 9 of 10); slurmd is not, and the node declares all CPUs.
set -euo pipefail

MACHINE="${1:-endeavor-linux}"
USER_NAME="${2:-$(ssh "$MACHINE" id -un)}"

as_root() { orbctl run -m "$MACHINE" -u root bash -c "$1"; }

# The remote script is a heredoc inside $(...): bash fails to parse it if it holds an unpaired apostrophe, even in a comment.
as_root "$(cat <<EOF
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
USER_NAME='$USER_NAME'
EOF
cat <<'EOF'
NODE="$(hostname -s)"
CLUSTER=endeavor
DB_PASS=slurmdbtest

# --- packages --------------------------------------------------------------
PKGS="slurm-wlm slurmdbd munge mariadb-server openssh-server"
if ! dpkg -s $PKGS >/dev/null 2>&1; then
  apt-get update -q
  apt-get install -y -q $PKGS
fi

# --- munge -------------------------------------------------------------------
if [ ! -s /etc/munge/munge.key ]; then
  mungekey --create --keyfile /etc/munge/munge.key
fi
chown munge:munge /etc/munge/munge.key
chmod 400 /etc/munge/munge.key
systemctl enable --now munge

# --- mariadb + slurmdbd --------------------------------------------------------
systemctl enable --now mariadb
mariadb -e "CREATE DATABASE IF NOT EXISTS slurm_acct_db;
CREATE USER IF NOT EXISTS 'slurm'@'localhost' IDENTIFIED BY '$DB_PASS';
ALTER USER 'slurm'@'localhost' IDENTIFIED BY '$DB_PASS';
GRANT ALL ON slurm_acct_db.* TO 'slurm'@'localhost';
FLUSH PRIVILEGES;"

install -d -o slurm -g slurm -m 755 /etc/slurm /var/log/slurm /run/slurm \
  /var/lib/slurm /var/lib/slurm/slurmctld /var/lib/slurm/slurmd

cat > /etc/slurm/slurmdbd.conf <<CONF
AuthType=auth/munge
DbdHost=localhost
SlurmUser=slurm
LogFile=/var/log/slurm/slurmdbd.log
PidFile=/run/slurm/slurmdbd.pid
StorageType=accounting_storage/mysql
StorageHost=localhost
StorageUser=slurm
StoragePass=$DB_PASS
StorageLoc=slurm_acct_db
CONF
chown slurm:slurm /etc/slurm/slurmdbd.conf
chmod 600 /etc/slurm/slurmdbd.conf
systemctl enable slurmdbd
systemctl restart slurmdbd
for _ in $(seq 30); do
  (exec 3<>/dev/tcp/127.0.0.1/6819) 2>/dev/null && break
  sleep 1
done

# --- slurm.conf ------------------------------------------------------------------
# Hardware line from slurmd -C, with 512 MB of RealMemory held back for the OS.
HW="$(slurmd -C | head -1)"
MEM="$(sed -n 's/.*RealMemory=\([0-9]*\).*/\1/p' <<<"$HW")"
MEM=$((MEM - 512))
CPUS="$(sed -n 's/.*CPUs=\([0-9]*\).*/\1/p' <<<"$HW")"
NODELINE="$(sed -e "s/^NodeName=[^ ]*/NodeName=$NODE/" -e "s/RealMemory=[0-9]*/RealMemory=$MEM/" <<<"$HW")"

cat > /etc/slurm/slurm.conf.new <<CONF
ClusterName=$CLUSTER
SlurmctldHost=$NODE
SlurmUser=slurm
AuthType=auth/munge
StateSaveLocation=/var/lib/slurm/slurmctld
SlurmdSpoolDir=/var/lib/slurm/slurmd
SlurmctldPidFile=/run/slurm/slurmctld.pid
SlurmdPidFile=/run/slurm/slurmd.pid
SlurmctldLogFile=/var/log/slurm/slurmctld.log
SlurmdLogFile=/var/log/slurm/slurmd.log
ReturnToService=2

SchedulerType=sched/backfill
SelectType=select/cons_tres
SelectTypeParameters=CR_Core_Memory
# Without a default, a job that omits --mem is given all memory on the node and nothing else fits.
DefMemPerCPU=$((MEM / CPUS))
ProctrackType=proctrack/cgroup
TaskPlugin=task/cgroup,task/affinity

MinJobAge=600
KillWait=10
OverTimeLimit=0

JobAcctGatherType=jobacct_gather/linux
JobAcctGatherFrequency=30
AccountingStorageType=accounting_storage/slurmdbd
AccountingStorageHost=localhost
JobCompType=jobcomp/none

$NODELINE State=UNKNOWN
PartitionName=shared Nodes=$NODE Default=YES MaxTime=8:00:00 DefaultTime=1:00:00 State=UP OverSubscribe=NO
PartitionName=short Nodes=$NODE MaxTime=1:00:00 DefaultTime=0:10:00 State=UP OverSubscribe=NO
CONF

# Swap is constrained too: the machine has swap, so a RAM limit alone lets jobs swap instead of hitting OOM.
cat > /etc/slurm/cgroup.conf.new <<CONF
CgroupPlugin=cgroup/v2
ConstrainCores=yes
ConstrainRAMSpace=yes
ConstrainSwapSpace=yes
AllowedSwapSpace=0
CONF

CHANGED=0
cmp -s /etc/slurm/slurm.conf.new /etc/slurm/slurm.conf || CHANGED=1
cmp -s /etc/slurm/cgroup.conf.new /etc/slurm/cgroup.conf || CHANGED=1
mv /etc/slurm/cgroup.conf.new /etc/slurm/cgroup.conf
chmod 644 /etc/slurm/cgroup.conf
mv /etc/slurm/slurm.conf.new /etc/slurm/slurm.conf
chmod 644 /etc/slurm/slurm.conf

if ! sacctmgr -n -P list cluster format=cluster | grep -qx "$CLUSTER"; then
  sacctmgr -i add cluster "$CLUSTER"
fi

systemctl enable slurmctld slurmd
if [ "$CHANGED" = 1 ] || ! systemctl is-active -q slurmctld || ! systemctl is-active -q slurmd; then
  systemctl restart slurmctld slurmd
fi
for _ in $(seq 30); do scontrol ping >/dev/null 2>&1 && break; sleep 1; done
# Bring the node back if an earlier run left it down or drained.
sleep 2
if sinfo -h -N -o '%t' | grep -qE 'down|drain'; then
  scontrol update NodeName="$NODE" State=RESUME || true
fi

# --- ssh from the machine to itself ------------------------------------------------------
systemctl enable --now ssh
HOME_DIR="$(getent passwd "$USER_NAME" | cut -d: -f6)"
runuser -u "$USER_NAME" -- bash -s "$NODE" <<'USERSH'
set -euo pipefail
NODE="$1"
cd ~
install -d -m 700 .ssh
[ -f .ssh/id_ed25519_node ] || ssh-keygen -q -t ed25519 -N '' -C "node-self" -f .ssh/id_ed25519_node
touch .ssh/authorized_keys .ssh/known_hosts .ssh/config
chmod 600 .ssh/authorized_keys .ssh/config
grep -qxF "$(cat .ssh/id_ed25519_node.pub)" .ssh/authorized_keys || cat .ssh/id_ed25519_node.pub >> .ssh/authorized_keys
for h in "$NODE" localhost; do
  ssh-keygen -F "$h" -f .ssh/known_hosts >/dev/null || ssh-keyscan -t ed25519 "$h" 2>/dev/null >> .ssh/known_hosts
done
if ! grep -q '^# slurm-setup: self' .ssh/config; then
  printf '# slurm-setup: self\nHost %s localhost\n  IdentityFile ~/.ssh/id_ed25519_node\n  IdentitiesOnly yes\n\n' "$NODE" | cat - .ssh/config > .ssh/config.tmp
  mv .ssh/config.tmp .ssh/config
  chmod 600 .ssh/config
fi
USERSH

echo "node=$NODE"
grep -E '^NodeName' /etc/slurm/slurm.conf
EOF
)"

ssh "$MACHINE" "sinfo -h -o '%P %l %c %m %t'; ssh -o BatchMode=yes \$(hostname -s) hostname"
