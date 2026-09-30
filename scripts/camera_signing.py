"""Validate camera provisioning authorization before embedding or signing."""
import datetime
import fnmatch


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
