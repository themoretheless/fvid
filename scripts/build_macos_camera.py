#!/usr/bin/env python3
"""Build (but never install) the FVid camera app and embedded system extension."""
import argparse
import json
from camera_signing import validated_entitlements, signing_fingerprint, validate_signing_authorization
import pathlib
import plistlib
import re
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', type=pathlib.Path, required=True, help='New .app path; existing paths are rejected')
p.add_argument('--team-id', help='Apple development team ID, required for signing')
p.add_argument('--host-profile', type=pathlib.Path, help='Host provisioning profile to embed')
p.add_argument('--extension-profile', type=pathlib.Path, help='Camera extension provisioning profile to embed')
p.add_argument('--sign', help='Explicit codesign identity; omitted builds an unsigned development bundle')
args = p.parse_args()
if args.output.suffix != '.app':
    p.error('--output must end in .app')
if args.team_id and not re.fullmatch(r'[A-Z0-9]{10}', args.team_id):
    p.error('--team-id must contain ten uppercase letters/digits')
if args.sign and not args.team_id:
    p.error('--sign requires --team-id')
profiles = []
if args.host_profile or args.extension_profile:
    if not args.sign or not args.host_profile or not args.extension_profile:
        p.error('provisioning requires --sign and both --host-profile/--extension-profile')
    listing = subprocess.run(['security', 'find-identity', '-v', '-p', 'codesigning'],
                             capture_output=True, text=True, check=True).stdout
    try:
        fingerprint = signing_fingerprint(args.sign, listing)
    except ValueError as error:
        p.error(str(error))
    hardware = subprocess.run(['system_profiler', 'SPHardwareDataType', '-json'],
                              capture_output=True, check=True)
    overview = json.loads(hardware.stdout)['SPHardwareDataType'][0]
    # Apple silicon registration uses Provisioning UDID, not Hardware UUID.
    device_id = overview.get('provisioning_UDID') or overview.get('platform_UUID')
    args.sign = fingerprint
    for profile, identifier in ((args.host_profile, 'org.fvid.camera'),
                                (args.extension_profile, 'org.fvid.camera.extension')):
        decoded = subprocess.run(['security', 'cms', '-D', '-i', str(profile)],
                                 capture_output=True, check=True)
        data = plistlib.loads(decoded.stdout)
        try:
            entitlements = validated_entitlements(data, args.team_id, identifier)
            validate_signing_authorization(data, fingerprint, device_id)
        except ValueError as error:
            p.error(str(error))
        profiles.append((profile, entitlements, identifier))
app = args.output.resolve()
app.mkdir(parents=True, exist_ok=False)
ext = app / 'Contents/Library/SystemExtensions/org.fvid.camera.extension.systemextension'
for bundle in (app, ext):
    (bundle / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
for bundle, (profile, _, _) in zip((app, ext), profiles):
    shutil.copyfile(profile, bundle / 'Contents/embedded.provisionprofile')
platform = ROOT / 'platform/macos'
shutil.copyfile(platform / 'CameraHost/Info.plist', app / 'Contents/Info.plist')
# Unsigned bundles are compile/structure artifacts only; LOCAL is not a signing team.
prefix = (args.team_id + '.') if args.team_id else 'LOCAL.'
ext_info = {
    'CFBundleIdentifier': 'org.fvid.camera.extension',
    'CFBundleName': 'FVid Camera Extension',
    'CFBundleExecutable': 'FVidCameraExtension',
    'CFBundlePackageType': 'SYSX',
    'CFBundleShortVersionString': '0.1.0',
    'CFBundleVersion': '1',
    'LSMinimumSystemVersion': '12.3',
    'CMIOExtension': {'CMIOExtensionMachServiceName': prefix + 'org.fvid.camera.extension'},
    'NSCameraUsageDescription': 'FVid publishes decoded video as a virtual camera.',
}
with (ext / 'Contents/Info.plist').open('wb') as f:
    plistlib.dump(ext_info, f)
cache = ROOT / 'target/camera-swift-cache'
cache.mkdir(parents=True, exist_ok=True)
common = ['xcrun', 'swiftc', '-warnings-as-errors', '-module-cache-path', str(cache), str(platform / 'Shared/CameraFormat.swift')]
ffi = ROOT / 'crates/fvid-camera-ffi'
subprocess.run(['cargo', 'build', '--locked', '--offline', '--manifest-path', str(ffi / 'Cargo.toml')], check=True)
bridge = ['-import-objc-header', str(platform / 'CameraHost/FVidCamera.h'), str(ffi / 'target/debug/libfvid_camera_ffi.a')]

subprocess.run(common + bridge + [str(platform / 'CameraExtension/PixelPool.swift'), str(platform / 'CameraHost/CameraSession.swift'), str(platform / 'CameraHost/NativeVideoSource.swift'), str(platform / 'CameraHost/CameraProducer.swift'), str(platform / 'CameraHost/main.swift'), '-o', str(app / 'Contents/MacOS/FVidCamera')], check=True)
subprocess.run(common + [str(platform / 'CameraExtension' / name) for name in ('PixelPool.swift', 'CameraProvider.swift', 'CameraSink.swift', 'main.swift')]
               + ['-o', str(ext / 'Contents/MacOS/FVidCameraExtension')], check=True)
for bundle in (app, ext):
    subprocess.run(['plutil', '-lint', str(bundle / 'Contents/Info.plist')], check=True)
if args.sign:
    for bundle, entitlements in ((ext, platform / 'CameraExtension/CameraExtension.entitlements'),
                                 (app, platform / 'CameraHost/CameraHost.entitlements')):
        signing_entitlements = entitlements
        if profiles:
            _, authorized, identifier = profiles[0 if bundle == app else 1]
            with entitlements.open('rb') as stream:
                values = plistlib.load(stream)
            for key in ('com.apple.application-identifier', 'application-identifier',
                        'com.apple.developer.team-identifier'):
                if key in authorized:
                    values[key] = authorized[key]
            signing_entitlements = cache / ('host-signing.plist' if bundle == app else 'extension-signing.plist')
            with signing_entitlements.open('wb') as stream:
                plistlib.dump(values, stream)
        subprocess.run(['codesign', '--force', '--options', 'runtime', '--sign', args.sign,
                        '--entitlements', str(signing_entitlements), str(bundle)], check=True)
        subprocess.run(['codesign', '--verify', '--strict', str(bundle)], check=True)
        signature = subprocess.run(['codesign', '-d', '-v', str(bundle)],
                                   capture_output=True, text=True, check=True)
        team = re.search(r'^TeamIdentifier=(.+)$', signature.stderr, re.MULTILINE)
        if team is None or team.group(1) != args.team_id:
            raise RuntimeError('Signing identity TeamIdentifier does not match --team-id; '
                               'bundle is not ready for activation')
print(f'Built {app}; ' + ('signed, not installed or notarized' if args.sign else 'unsigned: not installable'))
