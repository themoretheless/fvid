import datetime
import unittest
from camera_signing import validated_entitlements

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

if __name__=='__main__': unittest.main()
