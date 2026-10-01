"""Validate camera provisioning authorization before embedding or signing."""
import datetime
import fnmatch
import hashlib
import re


def validated_entitlements(data, team, identifier, now=None):
    authorized = dict(data.get('Entitlements', {}))
    pattern = authorized.get('com.apple.application-identifier', authorized.get('application-identifier', ''))
    if team not in data.get('TeamIdentifier', []):
        raise ValueError('provisioning team does not match ' + identifier)
    matches = [prefix + '.' + identifier for prefix in data.get('ApplicationIdentifierPrefix', [])
               if fnmatch.fnmatchcase(prefix + '.' + identifier, pattern)]
    if not matches:
        raise ValueError('provisioning application identifier does not match ' + identifier)
    if authorized.get('com.apple.developer.team-identifier', team) != team:
        raise ValueError('profile team entitlement does not match signing team')
    expiry = data.get('ExpirationDate')
    now = now or datetime.datetime.now(datetime.timezone.utc)
    if not isinstance(expiry, datetime.datetime) or expiry.replace(tzinfo=datetime.timezone.utc) <= now:
        raise ValueError('provisioning profile is expired or missing expiration')
    if identifier == 'org.fvid.camera' and authorized.get('com.apple.developer.system-extension.install') is not True:
        raise ValueError('host profile does not authorize system-extension installation')
    for key in ('com.apple.application-identifier', 'application-identifier'):
        if key in authorized:
            authorized[key] = matches[0]
    return authorized


def signing_fingerprint(identity, listing):
    """Resolve exactly one valid code signing identity to its certificate SHA-1."""
    identities = re.findall(r'^\s*\d+\) ([0-9A-Fa-f]{40}) "([^"\n]+)"', listing, re.MULTILINE)
    matches = {digest.upper() for digest, name in identities
               if identity.upper() == digest.upper() or identity == name}
    if len(matches) != 1:
        raise ValueError('signing identity must match exactly one valid certificate; use its SHA-1')
    return matches.pop()


def validate_signing_authorization(data, fingerprint, device_id):
    """Check the profile certificate and local device allowlists before building."""
    certificates = data.get('DeveloperCertificates', [])
    if not any(isinstance(cert, bytes) and hashlib.sha1(cert).hexdigest().upper() == fingerprint.upper()
               for cert in certificates):
        raise ValueError('provisioning profile does not authorize the signing certificate')
    if data.get('ProvisionsAllDevices') is True:
        return
    devices = data.get('ProvisionedDevices', [])
    if not device_id or not any(isinstance(device, str) and device.upper() == device_id.upper()
                                for device in devices):
        raise ValueError('provisioning profile does not authorize this Mac provisioning UDID')
