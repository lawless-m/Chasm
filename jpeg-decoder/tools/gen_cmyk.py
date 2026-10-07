"""Generate CMYK and YCCK JPEGs from a PPM for the corpus."""
import ctypes, sys
from PIL import Image

src, out = sys.argv[1], sys.argv[2]
cmyk = Image.open(src).convert("CMYK")
w, h = cmyk.size

# Pillow: Adobe CMYK (transform 0)
for q, ss in ((90, 0), (75, 2)):
    cmyk.save(f"{out}/cmyk-pillow-q{q}-{['444','422','420'][ss]}.jpg", quality=q, subsampling=ss)

# TurboJPEG: a CMYK pixel buffer compresses to YCCK (Adobe transform 2)
tj = ctypes.CDLL("libturbojpeg.so.0")
tj.tjInitCompress.restype = ctypes.c_void_p
tj.tjCompress2.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                           ctypes.c_int, ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte)),
                           ctypes.POINTER(ctypes.c_ulong), ctypes.c_int, ctypes.c_int, ctypes.c_int]
tj.tjFree.argtypes = [ctypes.c_void_p]
TJPF_CMYK, TJSAMP = 11, {"444": 0, "422": 1, "420": 2, "440": 4, "411": 5}
raw = cmyk.tobytes()
h_ = tj.tjInitCompress()
for name, ss in TJSAMP.items():
    buf, size = ctypes.POINTER(ctypes.c_ubyte)(), ctypes.c_ulong(0)
    if tj.tjCompress2(h_, raw, w, 0, h, TJPF_CMYK, ctypes.byref(buf), ctypes.byref(size), ss, 85, 0) != 0:
        sys.exit(f"tjCompress2 failed for {name}")
    open(f"{out}/ycck-tj-q85-{name}.jpg", "wb").write(ctypes.string_at(buf, size.value))
    tj.tjFree(buf)
