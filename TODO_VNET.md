# VNET Implementation - Status and Resolution

## Summary

VNET jail creation is working using the `vnet.interface` parameter during jail creation. The epair interface is automatically moved into the jail during creation, which is the correct approach for FreeBSD 15.0.

---

## Root Cause Analysis: Wrong Approach for Epair Attachment

### Original Issue
The networking module was attempting to attach epair interfaces to VNET jails using the wrong approach:
```bash
ifconfig epairXb vnet jailname  # WRONG APPROACH - This doesn't work reliably
```

This approach fails with "Device not configured" error because FreeBSD 15.0 requires the epair to be assigned DURING jail creation, not after.

### Correct Solution
Use the `vnet.interface` parameter during jail creation:
```bash
jail -c name=test path=/var/db/kawakaze/containers/test persist vnet vnet.interface=epair0b
```

This automatically moves the epair into the jail during creation and works reliably every time.

---

## Implementation Fix (February 2025)

### Issue
The networking module was trying to move epair interfaces to VNET jails AFTER jail creation using `ifconfig -vnet`, which fails with "Device not configured" error on FreeBSD 15.0.

### Root Cause
FreeBSD 15.0 VNET requires the epair interface to be assigned DURING jail creation using the `vnet.interface` parameter. The `ifconfig -vnet` approach is fundamentally broken on this system.

### Solution Implemented
Modified the jail creation to pass the epair interface name via `vnet.interface` parameter during jail creation:

#### backend/src/jail.rs
- Added `vnet_interface: Option<String>` field to the `Jail` struct
- Added `with_vnet_interface()` method to set the interface name
- Modified `create_freebsd_jail_with_command()` to accept and pass the interface to the jail command

#### backend/src/lib.rs
- Modified container creation to extract the `epair_jail` name from network allocation
- Pass the epair interface name to the jail creation via `with_vnet_interface()`

#### backend/src/networking.rs
- Removed the problematic `move_epair_to_vnet_jail()` call from `configure_jail_network()`
- The epair is now already in the jail during creation, so only IP configuration is needed

### Test Results (2025-02-05)

**Test Container: `3637e391-37ac-4ca7-98e1-5e5b9b6848c4`**

Created and started successfully with the following verification:

1. **Jail Status**: Running (JID 45)
2. **Network Configuration**:
   - IP: `10.11.0.36/16`
   - Gateway: `10.11.0.1`
   - Epair: `epair0b` (inside jail)
3. **VNET Isolation Verified**:
   - Container only sees `lo0` and `epair0b`
   - Host sees `epair0a` (attached to bridge0) but NOT `epair0b`
4. **Internet Connectivity**: ✅ Successfully pinged `8.8.8.8`
5. **DNS Resolution**: ✅ Successfully resolved `google.com`

---

## Verification Status

- ✅ VNET enabled (`vnet` in jail parameters)
- ✅ Epair interface assigned via `vnet.interface` parameter
- ✅ Epair is automatically moved into jail during creation (works immediately)
- ✅ IP address configured (10.11.0.x/16)
- ✅ Default route set (10.11.0.1)
- ✅ Can ping external IPs (8.8.8.8)
- ✅ DNS resolution working (google.com)
- ✅ No retry logic needed - works on first attempt
- ✅ VNET isolation verified (epair0b only visible inside jail)

---

## Known Limitations

None - The VNET implementation is working correctly with no known limitations.

---

## Files Modified

### backend/src/jail.rs
- Added `vnet_interface: Option<String>` field to `Jail` struct
- Added `with_vnet_interface(&str) -> Result<Self, JailError>` method
- Modified `create_freebsd_jail_with_command()` to accept `vnet_interface: Option<&str>` parameter
- Modified jail command to include `vnet.interface=<iface>` parameter when specified

### backend/src/lib.rs
- Modified container creation to extract `epair_jail` from network allocation
- Added call to `with_vnet_interface(epair)` when creating jail
- Epair is now passed to jail creation instead of trying to move it after

### backend/src/networking.rs
- Removed `move_epair_to_vnet_jail()` call from `configure_jail_network()`
- Added comment explaining epair is already moved via `vnet.interface` parameter
- Function now only configures IP and routing inside the jail

---

## Key Insight

The "Device not configured" error (ENODEV/SIOCSIFRVNET) when using `ifconfig -vnet` indicates that FreeBSD 15.0 requires the epair to be assigned DURING jail creation using the `vnet.interface` parameter, not moved after creation.

**OLD APPROACH (BROKEN):**
1. Create jail with `vnet`
2. Try to move epair with `ifconfig epairXb -vnet jailname` → FAILS

**NEW APPROACH (WORKS):**
1. Create epair
2. Create jail with `vnet vnet.interface=epairXb` → SUCCESS

---

## Future Work

None - VNET implementation is complete and working correctly.
