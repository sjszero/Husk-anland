#!/usr/bin/env python3
"""Embed the LinuxHub path shim inside the guest ld.so.

Input : the pristine glibc 2.39 aarch64 ld.so, the shim DSO, the _dl_init hook.
Output: an ld.so that carries the shim as one extra PT_LOAD, with every GOT slot
        the shim needs already resolved at build time and every pointer already
        relocated for the address the shim will actually be mapped at.

Why this is legal without a dynamic loader:
  * the shim is a position-independent ET_DYN whose 4K segments are already
    laid out at their own vaddrs, so moving it means moving the whole block by a
    constant: every byte keeps its internal offset;
  * the only relocations it needs are R_AARCH64_RELATIVE (pointers into its own
    data) and the GOT slots of its imports. We bake the former, and the
    _dl_init hook fills the latter from the loaded objects at startup.

Segment surgery: the loader's two PT_LOADs leave no room for a third, and the
phdr table has no spare slot, so PT_NOTE (the build-id note, debug-only) is
repurposed into the R|E PT_LOAD that carries the shim.
"""
import json
import struct
import sys

PT_LOAD, PT_DYNAMIC, PT_NOTE = 1, 2, 4
PT_GNU_RELRO, PT_GNU_EH_FRAME, PT_GNU_STACK = 0x6474e552, 0x6474e550, 0x6474e551

R_AARCH64_ABS64 = 257
R_AARCH64_GLOB_DAT = 1025
R_AARCH64_JUMP_SLOT = 1026
R_AARCH64_RELATIVE = 1027

STB_GLOBAL, STB_WEAK = 1, 2
STT_FUNC, STT_GNU_IFUNC = 2, 10

MAGIC = 0x4E52484B4F4F4B31  # "NRHOOK1"
PAGE = 0x1000

# ld.so constants this build has been checked against; a different glibc has to
# be re-checked rather than silently patched in the wrong place.
EXPECT_LOADER_BUILD = "glibc 2.39 aarch64"
LD_START_BL = 0x1AA74         # `bl _dl_init` inside _start
LD_DL_INIT = 0x4BB0           # its target == _dl_init
LD_TEXT_END = 0x41348         # end of the loader's last segment (vaddr)

# Deliberately not interposed: our own entry points and compiler glue.
NOT_INTERPOSED = {
    "nr_boot", "nr_resolve_name", "nr_get_main_map", "nr_set_main_map",
    "rust_eh_personality", "bcmp",
}


def align(v, a=PAGE):
    return (v + a - 1) & ~(a - 1)


class Elf:
    def __init__(self, path):
        self.data = bytearray(open(path, "rb").read())
        d = self.data
        assert d[:4] == b"\x7fELF" and d[4] == 2 and d[18:20] == b"\xb7\x00", \
            f"{path}: not an aarch64 ELF64"
        self.e_type = struct.unpack_from("<H", d, 0x10)[0]
        self.e_entry = struct.unpack_from("<Q", d, 0x18)[0]
        self.e_phoff = struct.unpack_from("<Q", d, 0x20)[0]
        self.e_shoff = struct.unpack_from("<Q", d, 0x28)[0]
        self.e_phentsize, self.e_phnum = struct.unpack_from("<HH", d, 0x36)
        self.e_shentsize, self.e_shnum = struct.unpack_from("<HH", d, 0x3A)
        assert self.e_phentsize == 56
        self.phdrs = []
        for i in range(self.e_phnum):
            o = self.e_phoff + i * 56
            p_type, p_flags = struct.unpack_from("<II", d, o)
            p_off, p_vaddr, p_paddr, p_filesz, p_memsz, p_align = \
                struct.unpack_from("<QQQQQQ", d, o + 8)
            self.phdrs.append(dict(idx=i, type=p_type, flags=p_flags, off=p_off,
                                   vaddr=p_vaddr, filesz=p_filesz,
                                   memsz=p_memsz, align=p_align))
        self.loads = [p for p in self.phdrs if p["type"] == PT_LOAD]

    def sections(self):
        out = {}
        d = self.data
        shstrndx = struct.unpack_from("<H", d, 0x3E)[0]
        strtab_off = struct.unpack_from("<Q", d, self.e_shoff + shstrndx * self.e_shentsize + 24)[0]
        for i in range(self.e_shnum):
            o = self.e_shoff + i * self.e_shentsize
            name_off, sh_type = struct.unpack_from("<II", d, o)
            sh_addr, sh_off, sh_size = struct.unpack_from("<QQQ", d, o + 16)
            nm = b""
            if strtab_off:
                end = d.index(b"\0", strtab_off + name_off)
                nm = bytes(d[strtab_off + name_off:end])
            out[nm.decode()] = dict(type=sh_type, addr=sh_addr, off=sh_off,
                                    size=sh_size, entsize=struct.unpack_from("<Q", d, o + 56)[0])
        return out

    def vaddr_to_off(self, vaddr):
        for p in self.loads:
            if p["vaddr"] <= vaddr < p["vaddr"] + p["filesz"]:
                return p["off"] + (vaddr - p["vaddr"])
        raise SystemExit(f"vaddr {vaddr:#x} is not backed by a file segment")


def cstr(d, off):
    end = d.index(b"\0", off)
    return bytes(d[off:end])


def main(shim_path, loader_path, hook_path, out_path):
    shim = Elf(shim_path)
    ld = Elf(loader_path)
    hook = bytearray(open(hook_path, "rb").read())
    sec = shim.sections()

    # ---------------------------------------------------------------- loader --
    assert ld.e_entry == 0x1AA40, f"unexpected loader entry {ld.e_entry:#x}"
    assert ld.loads[0]["vaddr"] == 0 and ld.loads[0]["off"] == 0, "loader LOAD0 not at 0"
    assert struct.unpack_from("<I", ld.data, LD_START_BL)[0] & 0xFC000000 == 0x94000000, \
        f"{LD_START_BL:#x} is not a `bl`"
    bl = struct.unpack_from("<I", ld.data, LD_START_BL)[0]
    imm26 = bl & 0x3FFFFFF
    if imm26 & 0x2000000:
        imm26 -= 1 << 26
    assert LD_START_BL + imm26 * 4 == LD_DL_INIT, \
        f"`bl` at {LD_START_BL:#x} targets {LD_START_BL + imm26*4:#x}, expected _dl_init"

    note = [p for p in ld.phdrs if p["type"] == PT_NOTE]
    assert len(note) == 1, "expected exactly one PT_NOTE to repurpose"
    note = note[0]
    assert note["filesz"] <= 0x40, "PT_NOTE is larger than expected"

    # ------------------------------------------------------------ shim block --
    span = 0
    for p in shim.loads:
        assert p["off"] == p["vaddr"], "shim segments are not offset==vaddr"
        assert p["off"] % PAGE == 0 and p["memsz"] >= p["filesz"]
        span = max(span, p["vaddr"] + p["memsz"])
    span = align(span)
    image = bytearray(span)
    segs = []
    for p in shim.loads:
        image[p["vaddr"]:p["vaddr"] + p["filesz"]] = \
            shim.data[p["off"]:p["off"] + p["filesz"]]
        segs.append(dict(vaddr=p["vaddr"], filesz=p["filesz"], memsz=p["memsz"],
                         flags=p["flags"]))

    # writable window: the shim's first PF_W segment, up to the end of the block
    rw = [p for p in shim.loads if p["flags"] & 2]
    assert rw, "shim has no writable segment"
    rw_start = rw[0]["vaddr"]
    rw_len = span - rw_start

    # ------------------------------------------------------- apply relocations --
    dynsym, dynstr = sec[".dynsym"], sec[".dynstr"]
    nsyms = dynsym["size"] // 24
    extern_entries = []
    relative_slots = []
    counter = dict(relative=0, ext=0, other=0)

    for relname in (".rela.dyn", ".rela.plt"):
        if relname not in sec:
            continue
        r = sec[relname]
        for i in range(r["size"] // 24):
            o = r["off"] + i * 24
            r_offset, r_info, r_addend = struct.unpack_from("<QQq", shim.data, o)
            r_sym = r_info >> 32          # ELF64 aarch64: sym in the high half
            r_type = r_info & 0xFFFFFFFF
            if r_type == 0:            # R_AARCH64_NONE
                continue
            if r_type == R_AARCH64_RELATIVE and r_sym == 0:
                # The link-time value only. R_AARCH64_RELATIVE means
                # "load bias + addend", and the bias is not known until the
                # kernel maps the loader, so nr_boot adds it at startup; the
                # slot address is recorded in the self-relocation table below.
                struct.pack_into("<Q", image, r_offset, V_BASE + r_addend)
                relative_slots.append(r_offset)
                counter["relative"] += 1
            elif r_type in (R_AARCH64_JUMP_SLOT, R_AARCH64_GLOB_DAT) and r_sym:
                # Includes calls to our own exported functions: a shared object's
                # globals go through the PLT because they are interposable, so the
                # slot needs the *runtime* address of our function and cannot be
                # baked relative. nr_boot resolves these against the interpose
                # table first, so they stay inside this object.
                s_name = struct.unpack_from("<I", shim.data, dynsym["off"] + r_sym * 24)[0]
                name = cstr(shim.data, dynstr["off"] + s_name)
                extern_entries.append((V_BASE + dynstr["addr"] + s_name, len(name),
                                       V_BASE + r_offset))
                counter["ext"] += 1
            elif r_type == R_AARCH64_ABS64:
                struct.pack_into("<Q", image, r_offset, V_BASE + r_addend)
                relative_slots.append(r_offset)
                counter["other"] += 1
            else:
                raise SystemExit(f"{relname}[{i}]: unsupported relocation {r_type}")

    # ------------------------------------------------------- interpose table --
    interpose_entries = []
    for i in range(1, nsyms):
        o = dynsym["off"] + i * 24
        st_name, st_info, st_other, st_shndx = struct.unpack_from("<IBBH", shim.data, o)
        st_value = struct.unpack_from("<Q", shim.data, o + 8)[0]
        bind, typ = st_info >> 4, st_info & 0xF
        if st_shndx == 0 or bind not in (STB_GLOBAL, STB_WEAK) or typ not in (STT_FUNC, STT_GNU_IFUNC):
            continue
        name = cstr(shim.data, dynstr["off"] + st_name)
        if name.decode() in NOT_INTERPOSED:
            continue
        interpose_entries.append((V_BASE + dynstr["addr"] + st_name, len(name),
                                  V_BASE + st_value))

    # name strings live in the shim's .dynstr, which must be inside the block
    assert dynstr["addr"] + dynstr["size"] <= span, ".dynstr is outside the block"

    # ------------------------------------------------------------ hook + data --
    hook_off = span
    data_off = hook_off + align(len(hook))
    tables = bytearray()

    DESC = 48                       # 6 x u64, see init-hook.S

    def table(entries):
        base = V_BASE + data_off + DESC + len(tables)
        blob = struct.pack("<Q", len(entries))
        for name_vaddr, name_len, payload in entries:
            blob += struct.pack("<QQQ", name_vaddr, name_len, payload)
        tables.extend(blob)
        return base

    ext_addr = table(extern_entries)
    interp_addr = table(interpose_entries)
    reloc_addr = table([(0, 0, V_BASE + off) for off in relative_slots])
    descriptor = struct.pack("<QQQQQQ", ext_addr, interp_addr,
                             V_BASE + shim_symbol("nr_boot", shim, sec, dynsym, dynstr),
                             V_BASE + rw_start, rw_len, reloc_addr)
    data = descriptor + bytes(tables)
    total = align(data_off + len(data))

    # patch the hook's magic header
    m = bytes(hook).index(struct.pack("<Q", MAGIC))
    struct.pack_into("<QQQ", hook, m + 8, V_BASE + hook_off, V_BASE + data_off, LD_DL_INIT)

    # ------------------------------------------------------------- write it out --
    out = bytearray(ld.data)
    if len(out) < F_BASE:
        out.extend(b"\0" * (F_BASE - len(out)))
    out.extend(b"\0" * (total - (len(out) - F_BASE)))

    assert all(b == 0 for b in out[F_BASE:]), "the space we are claiming is not empty"
    out[F_BASE:F_BASE + span] = image
    out[F_BASE + hook_off:F_BASE + hook_off + len(hook)] = hook
    out[F_BASE + data_off:F_BASE + data_off + len(data)] = data

    # PT_NOTE -> PT_LOAD(R|E)
    o = ld.e_phoff + note["idx"] * 56
    struct.pack_into("<IIQQQQQQ", out, o, PT_LOAD, 5, F_BASE, V_BASE, V_BASE,
                     total, total, PAGE)

    # `bl _dl_init` -> `bl nr_dl_init_hook`
    target = V_BASE + hook_off
    delta = (target - LD_START_BL) // 4
    assert -(1 << 25) <= delta < (1 << 25), "hook out of branch range"
    struct.pack_into("<I", out, LD_START_BL, 0x94000000 | (delta & 0x3FFFFFF))

    open(out_path, "wb").write(out)

    summary = dict(
        loader=EXPECT_LOADER_BUILD,
        shim_loads=[s["vaddr"] for s in segs],
        shim_span=span,
        hook_vaddr=target,
        descriptor_vaddr=V_BASE + data_off,
        rw=[rw_start, rw_len],
        externs=len(extern_entries),
        interposed=len(interpose_entries),
        relocations=counter,
        self_reloc_slots=len(relative_slots),
        file_offset=F_BASE,
        vaddr=V_BASE,
        total_load=total,
        out_size=len(out),
    )
    return summary


def shim_symbol(name, shim, sec, dynsym, dynstr):
    for i in range(dynsym["size"] // 24):
        o = dynsym["off"] + i * 24
        st_name = struct.unpack_from("<I", shim.data, o)[0]
        if cstr(shim.data, dynstr["off"] + st_name) == name.encode():
            return struct.unpack_from("<Q", shim.data, o + 8)[0]
    raise SystemExit(f"shim does not export {name}")


# Where the shim lands inside the loader. Fixed for this loader build and checked
# below: the loader's last segment ends at 0x41348, and the loader is 0x31cc0
# bytes long, so file 0x32000 and vaddr 0x42000 are both free and congruent.
F_BASE = 0x32000
V_BASE = 0x42000

if __name__ == "__main__":
    args = sys.argv[1:]
    if len(args) < 4:
        raise SystemExit("usage: nr-embed.py <shim.so> <loader.orig> <hook.bin> <out.so>")
    assert V_BASE >= align(LD_TEXT_END), "shim would overlap the loader's own segments"
    assert F_BASE % PAGE == V_BASE % PAGE, "offset/vaddr must be congruent"
    s = main(*args[:4])
    print(json.dumps(s, indent=2))
