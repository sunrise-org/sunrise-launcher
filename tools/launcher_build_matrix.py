#!/usr/bin/env python3
"""The launcher's release build matrix: one entry per platform it ships for, as JSON for GitHub Actions."""
import json
import sys

PLATFORMS = [
    dict(name='linux', os='ubuntu-22.04', blacksmith='blacksmith-16vcpu-ubuntu-2204', asset='sunrise-launcher-linux-x86_64.tar.gz', binary='sunrise-launcher'),
    dict(name='macos', mac=True, asset='sunrise-launcher-macos-arm64.zip', binary='sunrise-launcher', mac_arch='arm64'),
    dict(name='macos-x64', mac=True, target='x86_64-apple-darwin', asset='sunrise-launcher-macos-x64.zip', binary='sunrise-launcher', mac_arch='x64'),
    dict(name='windows', os='windows-latest', blacksmith='blacksmith-16vcpu-windows-2025', asset='sunrise-launcher-windows-x64.zip', binary='sunrise-launcher.exe', pe_arch='x64', setup_asset='sunrise-launcher-setup.exe', rustflags='-C target-feature=+crt-static'),
    dict(name='windows-arm64', os='windows-11-vs2026-arm', target='aarch64-pc-windows-msvc', asset='sunrise-launcher-windows-arm64.zip', binary='sunrise-launcher.exe', pe_arch='arm64', setup_asset='sunrise-launcher-setup-arm64.exe', rustflags='-C target-feature=+crt-static'),
]

# The platforms Sunrise ships the launcher for; macos-x64 and windows-arm64 build when they join.
SHIPPED = ('linux', 'macos', 'windows')

def matrix():
    return {'include': [dict(entry) for entry in PLATFORMS if entry['name'] in SHIPPED]}

def assets():
    """Every file a release publishes besides SHA256SUMS and its signature, sorted: what release.yml requires, no more and no less."""
    names = []
    for entry in matrix()['include']:
        names.append(entry['asset'])
        if entry['name'] == 'linux':
            names.append('sunrise-launcher-linux-x86_64.AppImage')
        if entry.get('mac'):
            names.append(f"sunrise-launcher-macos-{entry['mac_arch']}.dmg")
        if entry.get('setup_asset'):
            names.append(entry['setup_asset'])
    return sorted(names)

if __name__ == '__main__':
    print(json.dumps(assets() if sys.argv[1:] == ['--assets'] else matrix(), separators=(',', ':')))
