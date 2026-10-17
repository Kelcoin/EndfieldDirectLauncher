"""Extract the icon group of a PE executable into a standard .ico file.

The icon lives in the PE resource directory as two resource types: RT_GROUP_ICON
(14) holds the ICONDIR/ICONDIRENTRY table, RT_ICON (3) holds the raw image bytes
for each entry. This rebuilds a .ico from those two pieces, keeping every size
the source file carries.

Usage: python extract-icon.py <exe> <out.ico>
"""

import struct
import sys


def u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


def read_sections(b):
    pe = u32(b, 0x3C)
    if b[pe:pe + 4] != b"PE\0\0":
        raise SystemExit("not a PE file")
    nsec = u16(b, pe + 6)
    opt = pe + 24
    dd = opt + (112 if u16(b, opt) == 0x20B else 96)  # data directories start here
    rsrc_rva = u32(b, dd + 16)                        # entry 2 = resource table
    first = opt + u16(b, pe + 20)
    secs = []
    for i in range(nsec):
        h = first + i * 40
        secs.append((u32(b, h + 12), max(u32(b, h + 8), u32(b, h + 16)), u32(b, h + 20)))
    return secs, rsrc_rva


def rva_to_off(secs, rva):
    for va, size, ptr in secs:
        if va <= rva < va + size:
            return ptr + (rva - va)
    raise ValueError("rva %#x is outside every section" % rva)


def walk(b, base, rel, path, out):
    """Collect (path, data rva, size) for every resource leaf.

    `base` is the file offset of the resource directory; every offset inside the
    directory tree is relative to it.
    """
    off = base + rel
    n_named, n_id = u16(b, off + 12), u16(b, off + 14)
    p = off + 16
    for _ in range(n_named + n_id):
        name, sub = u32(b, p), u32(b, p + 4)
        p += 8
        ident = name & 0x7FFFFFFF
        if sub & 0x80000000:
            walk(b, base, sub & 0x7FFFFFFF, path + [ident], out)
        else:
            ent = base + (sub & 0x7FFFFFFF)  # IMAGE_RESOURCE_DATA_ENTRY
            out.append((tuple(path + [ident]), u32(b, ent), u32(b, ent + 4)))


def main():
    exe, out_path = sys.argv[1], sys.argv[2]
    b = open(exe, "rb").read()
    secs, rsrc_rva = read_sections(b)
    if not rsrc_rva:
        raise SystemExit("%s has no resource directory" % exe)

    leaves = []
    walk(b, rva_to_off(secs, rsrc_rva), 0, [], leaves)

    groups = {p[1]: (r, s) for p, r, s in leaves if p[0] == 14}
    icons = {p[1]: (r, s) for p, r, s in leaves if p[0] == 3}
    if not groups:
        raise SystemExit("no RT_GROUP_ICON in %s" % exe)
    gid = sorted(groups)[0]
    if len(groups) > 1:
        print("note: %d icon groups, using id %d" % (len(groups), gid))

    grva, _ = groups[gid]
    goff = rva_to_off(secs, grva)
    reserved, rtype, count = u16(b, goff), u16(b, goff + 2), u16(b, goff + 4)
    if reserved or rtype != 1 or count == 0:
        raise SystemExit("malformed group icon (reserved=%d type=%d count=%d)" % (reserved, rtype, count))

    entries = []  # (width, height, colors, planes, bits, bytes)
    for i in range(count):
        e = goff + 6 + i * 14
        w, h, colors = b[e], b[e + 1], b[e + 2]
        planes, bits = u16(b, e + 4), u16(b, e + 6)
        nbytes, nid = u32(b, e + 8), u16(b, e + 12)
        if nid not in icons:
            raise SystemExit("group lists icon id %d but RT_ICON has none" % nid)
        irva, isize = icons[nid]
        if isize < nbytes:
            raise SystemExit("icon %d: resource is %d bytes, group claims %d" % (nid, isize, nbytes))
        ioff = rva_to_off(secs, irva)
        entries.append((w, h, colors, planes, bits, b[ioff:ioff + nbytes]))

    out = bytearray(struct.pack("<HHH", 0, 1, len(entries)))
    data = bytearray()
    offset = 6 + 16 * len(entries)
    for w, h, colors, planes, bits, img in entries:
        out += struct.pack("<BBBBHHII", w, h, colors, 0, planes, bits, len(img), offset)
        data += img
        offset += len(img)
    out += data
    open(out_path, "wb").write(out)

    print("extracted %d size(s) -> %s (%d bytes)" % (len(entries), out_path, len(out)))
    for w, h, colors, planes, bits, img in entries:
        print("  %3dx%-3d %2dbpp colors=%-3d %6d bytes" % (w or 256, h or 256, bits, colors, len(img)))


if __name__ == "__main__":
    main()
