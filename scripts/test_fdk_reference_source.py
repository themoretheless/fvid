"""Pure stdlib, synthetic archive regressions; no Git, network or FDK needed."""
import gzip
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import fdk_reference_source as source

HELLO_TREE = 'aaa96ced2d9a1c8e72c56b253a0e2fe78393feb7'


def archive(entries=None, mtime=1):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode='w') as tar:
        for name, data, mode in entries or [('hello.txt', b'hello\n', 0o644)]:
            member = tarfile.TarInfo(name)
            member.size, member.mode, member.mtime = len(data), mode, mtime
            tar.addfile(member, io.BytesIO(data))
    return gzip.compress(out.getvalue(), mtime=mtime)


class SourcePinTests(unittest.TestCase):
    def verify(self, raw):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'source.tar.gz'
            path.write_bytes(raw)
            with patch.object(source, 'TREE_SHA1', HELLO_TREE):
                return source.verify_source_archive(path)

    def test_git_blob_and_tree_match_independently_frozen_git_hash_object(self):
        self.assertEqual(source.git_object(b'blob', b'hello\n').hex(),
                         'ce013625030ba8dba906f756967f9e9ca394464a')
        self.assertEqual(source.archive_tree(archive())['git_tree_sha1'], HELLO_TREE)

    def test_git_directory_prefix_sorting_uses_slash_after_name(self):
        raw = archive([('a/hello.txt', b'hello\n', 0o644), ('a.c', b'hello\n', 0o644)])
        self.assertEqual(source.archive_tree(raw)['git_tree_sha1'],
                         '26a9bef4c781668a1e62201bc511af3cd3002000')

    def test_transport_and_tar_timestamps_change_digest_not_content_pin(self):
        a, b = self.verify(archive(mtime=1)), self.verify(archive(mtime=2))
        self.assertNotEqual(a['archive_sha256'], b['archive_sha256'])
        self.assertEqual(a['git_tree_sha1'], b['git_tree_sha1'])
        self.assertEqual((a['files'], a['source_bytes']), (1, 6))

    def test_source_changes_rename_addition_and_executable_bits_refuse(self):
        for entries in [
            [('hello.txt', b'Hello\n', 0o644)],
            [('renamed.txt', b'hello\n', 0o644)],
            [('hello.txt', b'hello\n', 0o755)],
            [('hello.txt', b'hello\n', 0o644), ('extra', b'x', 0o644)],
        ]:
            with self.subTest(entries=entries), self.assertRaisesRegex(ValueError, 'tree pin mismatch'):
                self.verify(archive(entries))

    def test_duplicate_and_escaping_paths_refuse(self):
        for name in ['../hello.txt', '/hello.txt', 'a/../hello.txt', './hello.txt', 'a//hello.txt']:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'source path'):
                source.archive_tree(archive([(name, b'hello\n', 0o644)]))
        with self.assertRaisesRegex(ValueError, 'source path'):
            source.archive_tree(archive([('a', b'x', 0o644), ('a', b'x', 0o644)]))

    def test_file_directory_collision_refuses(self):
        for entries in [[('a', b'x', 0o644), ('a/b', b'x', 0o644)],
                        [('a/b', b'x', 0o644), ('a', b'x', 0o644)]]:
            with self.assertRaisesRegex(ValueError, 'collision'):
                source.archive_tree(archive(entries))

    def test_links_refuse_before_extraction(self):
        for kind in [tarfile.SYMTYPE, tarfile.LNKTYPE]:
            out = io.BytesIO()
            with tarfile.open(fileobj=out, mode='w') as tar:
                member = tarfile.TarInfo('link')
                member.type, member.linkname = kind, '../outside'
                tar.addfile(member)
            with self.assertRaisesRegex(ValueError, 'member type'):
                source.archive_tree(out.getvalue())

    def test_archive_and_expanded_payload_admission_bounds(self):
        with patch.object(source, 'MAX_ARCHIVE', 1), self.assertRaisesRegex(ValueError, 'admission bound'):
            source.archive_tree(archive())
        with patch.object(source, 'MAX_SOURCE', 1), self.assertRaisesRegex(ValueError, 'payload bound'):
            source.archive_tree(archive())

    def test_gzip_expansion_is_bounded_before_tar_extended_headers(self):
        with patch.object(source, 'MAX_UNPACKED', 32), self.assertRaisesRegex(ValueError, 'expanded archive bound'):
            source.archive_tree(archive())


if __name__ == '__main__':
    unittest.main()
