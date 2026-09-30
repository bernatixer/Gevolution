#!/usr/bin/env python3
"""Adapt Kenney Nature Kit GLBs to Gevolution's lit, natural look.

The originals are stylized: unlit, fully metallic, mint-green foliage. This
rewrites only the JSON chunk of each GLB (materials), leaving geometry intact:
lit PBR, non-metallic, rough, with a natural palette. Idempotent.
"""
import glob, json, os, struct, sys

# sRGB colors; glTF factors are linear, converted in fix().
PALETTE = {
    "leafsGreen": [0.30, 0.52, 0.18], "leafsDark": [0.17, 0.38, 0.17], "grass": [0.38, 0.58, 0.22],
    "leafsFall": [0.86, 0.46, 0.14], "woodBark": [0.42, 0.28, 0.18], "woodBarkDark": [0.30, 0.20, 0.14],
    "woodBirch": [0.88, 0.85, 0.78], "woodInner": [0.80, 0.66, 0.48], "dirt": [0.48, 0.36, 0.26],
    "stone": [0.56, 0.56, 0.54], "colorPurple": [0.60, 0.45, 0.85], "colorRed": [0.85, 0.24, 0.24],
    "colorYellow": [0.98, 0.78, 0.28], "colorTan": [0.86, 0.66, 0.46],
}

def fix(path):
    b = bytearray(open(path, "rb").read())
    jlen = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20:20 + jlen])
    for m in j.get("materials", []):
        pbr = m.setdefault("pbrMetallicRoughness", {})
        rgb = PALETTE.get(m.get("name"))
        if rgb:
            pbr["baseColorFactor"] = [round(((c + 0.055) / 1.055) ** 2.4, 4) for c in rgb] + [1.0]
        pbr["metallicFactor"] = 0.0
        pbr["roughnessFactor"] = 0.85
        m.get("extensions", {}).pop("KHR_materials_unlit", None)
        if m.get("extensions") == {}:
            del m["extensions"]
    for key in ("extensionsUsed", "extensionsRequired"):
        if key in j:
            j[key] = [e for e in j[key] if e != "KHR_materials_unlit"]
            if not j[key]:
                del j[key]
    data = json.dumps(j, separators=(",", ":")).encode()
    data += b" " * ((4 - len(data) % 4) % 4)
    rest = b[20 + jlen:]
    out = bytearray(b[:12]) + struct.pack("<I", len(data)) + b"JSON" + data + rest
    out[8:12] = struct.pack("<I", len(out))
    open(path, "wb").write(out)

if __name__ == "__main__":
    root = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "../assets/client/models/nature")
    for p in sorted(glob.glob(os.path.join(root, "*.glb"))):
        fix(p)
    print("prepared", len(glob.glob(os.path.join(root, "*.glb"))), "models")
