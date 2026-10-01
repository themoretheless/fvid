import datetime
import unittest
import hashlib
from camera_signing import validated_entitlements, signing_fingerprint, validate_signing_authorization

class Profiles(unittest.TestCase):
    def profile(self):
        return {'TeamIdentifier':['TEAM123456'], 'ApplicationIdentifierPrefix':['OLDPREFIX1'],
                'ExpirationDate':datetime.datetime(2030,1,1), 'Entitlements':{
                    'com.apple.application-identifier':'OLDPREFIX1.org.fvid.camera',
                    'com.apple.developer.team-identifier':'TEAM123456',
                    'com.apple.developer.system-extension.install':True}}
    def test_legacy_prefix_is_independent_from_team(self):
        value=validated_entitlements(self.profile(),'TEAM123456','org.fvid.camera')
        self.assertEqual(value['com.apple.application-identifier'],'OLDPREFIX1.org.fvid.camera')
    def test_wildcard_resolves_to_concrete_bundle(self):
        value=self.profile();value['Entitlements']['com.apple.application-identifier']='OLDPREFIX1.*'
        self.assertEqual(validated_entitlements(value,'TEAM123456','org.fvid.camera.extension')['com.apple.application-identifier'],'OLDPREFIX1.org.fvid.camera.extension')
    def test_wrong_team_bundle_expiry_and_missing_install_authorization(self):
        for change in ['team','bundle','expiry','install','entitlement_team']:
            value=self.profile()
            if change=='team':value['TeamIdentifier']=['OTHERTEAM1']
            if change=='bundle':value['Entitlements']['com.apple.application-identifier']='OLDPREFIX1.org.other'
            if change=='expiry':value['ExpirationDate']=datetime.datetime(2000,1,1)
            if change=='install':value['Entitlements'].pop('com.apple.developer.system-extension.install')
            if change=='entitlement_team':value['Entitlements']['com.apple.developer.team-identifier']='OTHERTEAM1'
            with self.subTest(change=change),self.assertRaises(ValueError):
                validated_entitlements(value,'TEAM123456','org.fvid.camera')

class SigningAuthorization(unittest.TestCase):
    def test_exact_identity_resolution_rejects_ambiguous_names(self):
        a, b = 'A' * 40, 'B' * 40
        listing = f'  1) {a} "Apple Development: Example"\n  2) {b} "Apple Development: Example"\n'
        self.assertEqual(signing_fingerprint(a.lower(), listing), a)
        for name in ['Apple Development: Example', 'Example', '-', 'C' * 40]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                signing_fingerprint(name, listing)

    def test_certificate_and_device_are_both_required(self):
        cert = b'certificate DER fixture'
        fingerprint = hashlib.sha1(cert).hexdigest()
        profile = {'DeveloperCertificates': [cert], 'ProvisionedDevices': ['LOCAL-UDID']}
        validate_signing_authorization(profile, fingerprint, 'local-udid')
        for modified, digest, device in [
            ({**profile, 'DeveloperCertificates': []}, fingerprint, 'LOCAL-UDID'),
            (profile, 'A' * 40, 'LOCAL-UDID'),
            (profile, fingerprint, 'HARDWARE-UUID'),
            (profile, fingerprint, None),
            ({'DeveloperCertificates': [cert]}, fingerprint, 'LOCAL-UDID'),
        ]:
            with self.subTest(profile=modified, device=device), self.assertRaises(ValueError):
                validate_signing_authorization(modified, digest, device)

    def test_all_devices_still_requires_authorized_certificate(self):
        cert = b'certificate DER fixture'
        profile = {'DeveloperCertificates': [cert], 'ProvisionsAllDevices': True}
        validate_signing_authorization(profile, hashlib.sha1(cert).hexdigest(), None)
        with self.assertRaises(ValueError):
            validate_signing_authorization(profile, 'A' * 40, None)

if __name__=='__main__': unittest.main()
