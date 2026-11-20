"""Offline content pin for optional original AOSP FDK reference builds.

Gitiles tarballs include download-time mtimes. Verify the entire Git tree of
the fixed commit, including paths and executable bits, before extraction/build.
No production decoder or ordinary media test imports this module.
"""
import hashlib
import gzip
import io
from pathlib import Path, PurePosixPath
import tarfile

COMMIT = 'ee76460efbdb147e26d804c798949c23f174460b'
# Independently obtained from the pinned primary Gitiles commit object.
TREE_SHA1 = 'c3900d70bbdf4129d37f8cd5ab4a6589a34eb95b'
LEGACY_ARCHIVE_SHA256 = '4f41fba1e55dc554ccd0166f550d7c0c83ae0f22d7f2ed76001537f4b62f7325'
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_SOURCE = 128 * 1024 * 1024
MAX_UNPACKED = MAX_SOURCE + 8 * 1024 * 1024


def git_object(kind, data):
    return hashlib.sha1(kind + b' ' + str(len(data)).encode() + b'\0' + data).digest()


def archive_tree(payload):
    if len(payload) > MAX_ARCHIVE:
        raise ValueError('FDK source archive admission bound')
    if payload.startswith(b'\x1f\x8b'):
        with gzip.GzipFile(fileobj=io.BytesIO(payload)) as compressed:
            payload = compressed.read(MAX_UNPACKED + 1)
    if len(payload) > MAX_UNPACKED:
        raise ValueError('FDK expanded archive bound')
    root, seen, total, files = {}, set(), 0, 0
    with tarfile.open(fileobj=io.BytesIO(payload), mode='r:') as archive:
        for index, member in enumerate(archive):
            if index >= 10000:
                raise ValueError('FDK source archive member bound')
            name = member.name.rstrip('/')
            parts = PurePosixPath(name).parts
            if (not parts or PurePosixPath(name).is_absolute() or '..' in parts
                    or '/'.join(parts) != name or name in seen):
                raise ValueError('invalid or duplicate FDK source path')
            seen.add(name)
            node = root
            for part in parts[:-1]:
                node = node.setdefault(part, {})
                if not isinstance(node, dict):
                    raise ValueError('FDK source file/directory collision')
            leaf = parts[-1]
            if member.isdir():
                if not isinstance(node.setdefault(leaf, {}), dict):
                    raise ValueError('FDK source file/directory collision')
                continue
            if not member.isfile() or member.size < 0 or member.size > 16 * 1024 * 1024:
                raise ValueError('invalid FDK source member type/size')
            total += member.size
            if total > MAX_SOURCE or leaf in node:
                raise ValueError('FDK source payload bound or path collision')
            raw = archive.extractfile(member).read(member.size + 1)
            if len(raw) != member.size:
                raise ValueError('truncated FDK source member')
            mode = b'100755' if member.mode & 0o111 else b'100644'
            node[leaf] = (mode, git_object(b'blob', raw))
            files += 1

    def tree(node):
        rows = []
        for name, value in node.items():
            directory = isinstance(value, dict)
            mode, digest = (b'40000', tree(value)) if directory else value
            key = name.encode('utf-8')
            rows.append((key + (b'/' if directory else b''), mode + b' ' + key + b'\0' + digest))
        return git_object(b'tree', b''.join(value for _, value in sorted(rows)))

    return dict(git_tree_sha1=tree(root).hex(), files=files, source_bytes=total)


def verify_source_archive(path):
    path = Path(path)
    if path.stat().st_size > MAX_ARCHIVE:
        raise ValueError('FDK source archive admission bound')
    payload = path.read_bytes()
    result = archive_tree(payload)
    if result['git_tree_sha1'] != TREE_SHA1:
        raise ValueError('FDK source Git tree pin mismatch')
    return dict(commit=COMMIT, archive_sha256=hashlib.sha256(payload).hexdigest(), **result)
