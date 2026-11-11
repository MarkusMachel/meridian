"""Writing the Meridian array container (the format of the baked map; see bake_map.py):
magic "MRMP", version, kind, payload length, CRC-32, then named little-endian arrays."""
import struct
import zlib
from pathlib import Path

import numpy as np

DTYPES = {"u1": 1, "<u2": 2, "<u4": 3, "<i2": 4, "<i4": 5, "<u8": 6}


def write(out_file, kind, version, arrays):
    """`arrays`: name -> (numpy array, dtype string from DTYPES). Returns the file size."""
    payload = bytearray(struct.pack("<I", len(arrays)))
    for name, (arr, dt) in arrays.items():
        nb = name.encode("ascii")
        arr = np.ascontiguousarray(arr)
        payload += struct.pack("<B", len(nb)) + nb + struct.pack("<BQ", DTYPES[dt], arr.size) + arr.astype(dt, copy=False).tobytes()
    header = b"MRMP" + struct.pack("<IIQQ", version, kind, len(payload), zlib.crc32(bytes(payload)))
    Path(out_file).parent.mkdir(parents=True, exist_ok=True)
    Path(out_file).write_bytes(header + bytes(payload))
    return len(header) + len(payload)
