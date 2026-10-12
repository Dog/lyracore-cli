# Host operations

Host administration uses `lyracore-ops`, separate from the developer CLI. The previous
`lyracore service reconcile` command now prints the replacement command and exits without changes.

From a checkout of this CLI at the revision you want to install:

```bash
cargo install --locked --path . --bin lyracore-ops
```

Then, from the selected **Core** checkout, run the installed binary as root. Use its absolute
installed path if root's PATH does not contain your Cargo bin directory:

```bash
sudo "$HOME/.cargo/bin/lyracore-ops" service reconcile
```

The tool reads `deploy/systemd/spacetimedb-standalone.service` from that Core checkout.
Update and review the checkout separately, as its owner, before reconciliation. This command
does not fetch, reset, build the gateway, publish a module, or perform a schema migration.

Reconciliation performs these steps:

1. Verify root privileges with `id -u` and read the selected unit.
2. Check the unit's account, executable, data directory, log directory and required host tools.
   Missing prerequisites are named and refused; the tool does not create them.
3. Refuse another active service claiming the same data directory or listen address.
4. Install the unit into `/etc/systemd/system`, reload systemd, enable it and restart it.
5. Verify the effective start command, active state, descriptor limit and stderr destination
   against the tracked unit, including systemd overrides.

The database directory is checked for existence, never created, moved or deleted.
Every reconciliation restarts the node and causes a short outage. Failures identify which
step stopped; reconciliation is not a transaction that rolls back a failed restart.

`lyracore production status` remains a read-only developer command for an explicit topology.

The `deploy/` directory also contains the history-prune, disk-guard and diagnostic-capture
scripts and their systemd units. Their installation remains described by Core's
`docs/danger-zones.md` and `docs/operations/disk-policy.md`.
