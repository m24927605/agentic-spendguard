from __future__ import annotations

import base64
import gzip
import hashlib
from pathlib import Path
import tarfile
import zipfile


PROJECT_NAME = "spendguard-agentrust"
PACKAGE_NAME = "spendguard_agentrust"
VERSION = "0.1.0a1"
DIST_INFO = f"{PACKAGE_NAME}-{VERSION}.dist-info"
WHEEL_NAME = f"{PACKAGE_NAME}-{VERSION}-py3-none-any.whl"
SDIST_ROOT = f"{PACKAGE_NAME}-{VERSION}"
SDIST_NAME = f"{SDIST_ROOT}.tar.gz"
ZIP_TIMESTAMP = (2026, 7, 1, 0, 0, 0)
TAR_MTIME = 1782864000
RUNTIME_REQUIRES = [
    "agent-manifest==0.1.1",
    "agentrust-trace==0.2.0",
    "cryptography==49.0.0",
    "jsonschema==4.26.0",
]


def get_requires_for_build_wheel(config_settings=None):
    return []


def get_requires_for_build_sdist(config_settings=None):
    return []


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    dist_info = Path(metadata_directory) / DIST_INFO
    dist_info.mkdir(parents=True, exist_ok=True)
    (dist_info / "METADATA").write_bytes(_metadata_bytes())
    (dist_info / "WHEEL").write_bytes(_wheel_metadata_bytes())
    (dist_info / "RECORD").write_text("", encoding="utf-8")
    return DIST_INFO


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    wheel_path = Path(wheel_directory) / WHEEL_NAME
    entries = _wheel_entries()
    record_name = f"{DIST_INFO}/RECORD"
    record_data = _record_bytes(entries, record_name)
    all_entries = sorted([*entries, (record_name, record_data)], key=lambda item: item[0])

    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for arcname, data in all_entries:
            _zip_write(archive, arcname, data)

    return WHEEL_NAME


def build_sdist(sdist_directory, config_settings=None):
    sdist_path = Path(sdist_directory) / SDIST_NAME
    entries = _sdist_entries()
    with sdist_path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=TAR_MTIME) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for arcname, data in entries:
                    info = tarfile.TarInfo(arcname)
                    info.size = len(data)
                    info.mode = 0o644
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mtime = TAR_MTIME
                    archive.addfile(info, _BytesReader(data))
    return SDIST_NAME


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def _wheel_entries() -> list[tuple[str, bytes]]:
    root = _repo_root()
    entries: list[tuple[str, bytes]] = []
    for path in sorted((root / PACKAGE_NAME).glob("*.py")):
        if not _is_public_package_file(path):
            continue
        entries.append((f"{PACKAGE_NAME}/{path.name}", path.read_bytes()))
    entries.extend(
        [
            (f"{DIST_INFO}/METADATA", _metadata_bytes()),
            (f"{DIST_INFO}/WHEEL", _wheel_metadata_bytes()),
        ]
    )
    return sorted(entries, key=lambda item: item[0])


def _sdist_entries() -> list[tuple[str, bytes]]:
    root = _repo_root()
    source_paths = [
        root / "pyproject.toml",
        root / "packaging" / "agentrust_build_backend.py",
        *sorted((root / PACKAGE_NAME).glob("*.py")),
    ]
    entries: list[tuple[str, bytes]] = []
    for path in source_paths:
        if path.parent.name == PACKAGE_NAME and not _is_public_package_file(path):
            continue
        rel = path.relative_to(root).as_posix()
        entries.append((f"{SDIST_ROOT}/{rel}", path.read_bytes()))
    return sorted(entries, key=lambda item: item[0])


def _is_public_package_file(path: Path) -> bool:
    private_names = {
        "_" + "make_verify.py",
        "_" + "verify_internal.py",
    }
    return path.name not in private_names


def _metadata_bytes() -> bytes:
    lines = [
        "Metadata-Version: 2.3",
        f"Name: {PROJECT_NAME}",
        f"Version: {VERSION}",
        "Summary: Fixture-only SpendGuard AgenTrust verifier",
        "Requires-Python: >=3.12",
        *[f"Requires-Dist: {requirement}" for requirement in RUNTIME_REQUIRES],
        "",
        "",
    ]
    return "\n".join(lines).encode("utf-8")


def _wheel_metadata_bytes() -> bytes:
    return (
        "Wheel-Version: 1.0\n"
        "Generator: agentrust_build_backend\n"
        "Root-Is-Purelib: true\n"
        "Tag: py3-none-any\n"
        "\n"
    ).encode("utf-8")


def _record_bytes(records: list[tuple[str, bytes]], record_name: str) -> bytes:
    rows: list[str] = []
    for arcname, data in records:
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=")
        rows.append(f"{arcname},sha256={digest.decode('ascii')},{len(data)}")
    rows.append(f"{record_name},,")
    return ("\n".join(rows) + "\n").encode("utf-8")


def _zip_write(archive: zipfile.ZipFile, arcname: str, data: bytes) -> None:
    info = zipfile.ZipInfo(arcname, ZIP_TIMESTAMP)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o644 << 16
    archive.writestr(info, data)


class _BytesReader:
    def __init__(self, data: bytes):
        self._data = data
        self._offset = 0

    def read(self, size: int = -1) -> bytes:
        if size is None or size < 0:
            size = len(self._data) - self._offset
        chunk = self._data[self._offset : self._offset + size]
        self._offset += len(chunk)
        return chunk
