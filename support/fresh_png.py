# Minimal PNG writer for uint8 RGB/gray arrays (no external imaging dependency).
import zlib, struct, numpy as np
def write(path, img):
    img = np.ascontiguousarray(img.astype(np.uint8))
    if img.ndim == 2: img = np.repeat(img[..., None], 3, -1)
    raw = b''.join(b'\0' + row.tobytes() for row in img)
    def chunk(t, d): return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', img.shape[1], img.shape[0], 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b''))
