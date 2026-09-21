#!/usr/bin/env python3
"""Build (but never install) the FVid camera app and embedded system extension."""
import argparse
import pathlib
import plistlib
import re
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', type=pathlib.Path, required=True, help='New .app path; existing paths are rejected')
p.add_argument('--team-id', help='Apple development team ID, required for signing')
p.add_argument('--sign', help='Explicit codesign identity; omitted builds an unsigned development bundle')
args = p.parse_args()
if args.output.suffix != '.app':
    p.error('--output must end in .app')
if args.team_id and not re.fullmatch(r'[A-Z0-9]{10}', args.team_id):
    p.error('--team-id must contain ten uppercase letters/digits')
if args.sign and not args.team_id:
    p.error('--sign requires --team-id')
app = args.output.resolve()
app.mkdir(parents=True, exist_ok=False)
ext = app / 'Contents/Library/SystemExtensions/org.fvid.camera.extension.systemextension'
for bundle in (app, ext):
    (bundle / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
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
common = ['xcrun', 'swiftc', '-warnings-as-errors', '-module-cache-path', str(cache)]
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
        subprocess.run(['codesign', '--force', '--options', 'runtime', '--sign', args.sign,
                        '--entitlements', str(entitlements), str(bundle)], check=True)
        subprocess.run(['codesign', '--verify', '--strict', str(bundle)], check=True)
print(f'Built {app}; ' + ('signed, not installed or notarized' if args.sign else 'unsigned: not installable'))
