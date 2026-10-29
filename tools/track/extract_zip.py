"""Extract data from an untrusted ZIP into a new, empty directory under /tmp."""

import argparse
import shutil
import stat
import tempfile
import zipfile
from pathlib import Path, PurePosixPath


def extract(path):
    destination = Path(tempfile.mkdtemp(prefix="racecraft-assets-", dir="/tmp"))
    try:
        with zipfile.ZipFile(path) as archive:
            total = 0
            for member in archive.infolist():
                name = PurePosixPath(member.filename)
                kind = stat.S_IFMT(member.external_attr >> 16)
                if (
                    name.is_absolute()
                    or ".." in name.parts
                    or "\\" in member.filename
                    or kind not in (0, stat.S_IFREG, stat.S_IFDIR)
                ):
                    raise ValueError(f"unsafe archive member: {member.filename!r}")
                total += member.file_size
                if total > 4 * 1024**3:
                    raise ValueError("archive exceeds 4 GiB extraction budget")
                target = destination / name
                if member.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                # Exclusive creation rejects duplicate members as well.
                with archive.open(member) as source, target.open("xb") as output:
                    shutil.copyfileobj(source, output)
        return destination
    except BaseException:
        shutil.rmtree(destination)
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("zip")
    print(extract(parser.parse_args().zip))
