import hashlib,json,os,subprocess,socket,tempfile,unittest
from pathlib import Path
from unittest import mock
import generate_matroska_audio_sample_clock_regressions as generator
from matroska_sample_clock_reference import extract,load
class Fixtures(unittest.TestCase):
    def test_generated_codec_witnesses_match_recorded_hashes(self):
        base=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-audio-sample-clocks'
        manifest=json.loads((base/'manifest.json').read_text())
        self.assertEqual(len(manifest['cases']),6)
        for case in manifest['cases']:self.assertEqual(hashlib.sha256((base/case['file']).read_bytes()).hexdigest(),case['sha256'])

    def test_owned_generation_reproduces_all_six_movies_without_programs_or_network(self):
        with tempfile.TemporaryDirectory(prefix='fvid-clock-no-outputs-') as directory, \
             mock.patch.object(generator,'OUT',Path(directory)/'absent-output'), \
             mock.patch.dict(os.environ,{'PATH':''}), \
             mock.patch.object(subprocess,'Popen',side_effect=AssertionError('fixture generation spawned a program')), \
             mock.patch.object(socket,'socket',side_effect=AssertionError('fixture generation used the network')):
            files=generator.generate()
        self.assertEqual(set(files),{'manifest.json',*[name+'.mkv'for name in ['flac','alac','mp3','mp2','vorbis','ac3']]})
        for name,data in files.items():self.assertEqual(data,(generator.OUT/name).read_bytes(),name)

    def test_every_original_audio_payload_configuration_and_clock_is_preserved(self):
        counts={'flac':12,'alac':3,'mp3':11,'mp2':10,'vorbis':13,'ac3':8}
        for original,private,packets in load():
            self.assertEqual(len(packets),counts[original['name']])
            movie=(generator.OUT/(original['name']+'.mkv')).read_bytes()
            actual,files=extract(movie,original['name'],original['reference_encoder'])
            for key in ['codec','sample_rate','channels','bit_depth','codec_delay_ns','seek_preroll_ns',
                        'default_duration_ns','timestamp_scale_ns','duration_ticks','packets']:
                self.assertEqual(actual[key],original[key],f'{original["name"]}: {key}')
            self.assertEqual(files[actual['payload_file']],b''.join(packets))
            self.assertEqual(files[actual['private_file']],private)
            self.assertEqual(actual['payload_sha256'],original['payload_sha256'])
