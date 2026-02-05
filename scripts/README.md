# Kawakaze Setup Scripts

This directory contains utility scripts to help set up and configure Kawakaze on FreeBSD.

## Scripts

### setup-pf.sh

Automates the configuration of pf (Packet Filter) for Kawakaze container networking.

**Usage:**
```bash
# Basic usage - interactive mode
sudo ./scripts/setup-pf.sh

# Force create/overwrite pf.conf
sudo ./scripts/setup-pf.sh --force-pfconf
```

**What it does:**
1. Checks if pf is enabled at boot and currently running
2. Enables pf at boot using `sysrc pf_enable=YES`
3. Starts pf service if not already running
4. Optionally creates a basic `/etc/pf.conf` with:
   - Automatic external interface detection
   - Kawakaze pf anchors
   - Basic security rules
5. Shows final pf status and rules

**Requirements:**
- Must be run as root
- FreeBSD system with pf installed (default on FreeBSD)

**Example output:**
```
==========================================
  Kawakaze pf Setup Script
==========================================

[INFO] Checking pf status...
[WARN] pf is not enabled at boot
[WARN] pf is not currently running

[INFO] Enabling pf at boot...
pf enabled in /etc/rc.conf
[INFO] pf enabled at boot

[INFO] Starting pf...
Enabling pf...
pf enabled
[INFO] pf started successfully

Create basic pf.conf? (y/N): y
[INFO] Creating basic pf.conf for Kawakaze...
[INFO] Created pf.conf with external interface: vtnet0
[INFO] Loading new pf.conf...

[INFO] === pf Status ===
Status: Enabled for 0 days 00:00:05
...

[INFO] === Setup Complete ===

pf is now configured for Kawakaze.
```

## Troubleshooting

### pf fails to start
```bash
# Check pf logs
pfctl -s info

# Enable debug mode
pfctl -x debug

# Check syntax of pf.conf
pfctl -nf /etc/pf.conf
```

### Check Kawakaze anchors
```bash
# View NAT rules
pfctl -a kawakaze -s nat

# View port forwarding rules
pfctl -a kawakaze_forwarding -s nat

# View all rules
pfctl -a kawakaze -s rules
```

### Restore from backup
If the script created a backup of your pf.conf:
```bash
# List backups
ls -la /etc/pf.conf.kawakaze.backup*

# Restore backup
cp /etc/pf.conf.kawakaze.backup /etc/pf.conf
pfctl -f /etc/pf.conf
```
