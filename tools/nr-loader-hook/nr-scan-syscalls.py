#!/usr/bin/env python3
"""Locate the optional-accelerator syscall sites in a glibc image, by pattern.

    nr-scan-syscalls.py report  <elf> [--expect NAME=COUNT ...]
    nr-scan-syscalls.py patch   <elf> <out> [--expect NAME=COUNT ...]

Why a scan and not an offset table: the sites move between glibc builds, and a
stale offset silently corrupts whatever happens to be there. This script instead

  * reads the image's own version definitions (.gnu.version_d) and refuses to
    touch a build it was not told to expect;
  * finds `svc #0` sites and walks *backwards* to the `mov x8,#NR` that selects
    the syscall (glibc sets the argument registers after x8, so they are not
    adjacent), stopping at anything that would break the association;
  * keeps only the syscalls on the accelerator list (set_robust_list, rseq - the
    ones the caller may treat as optional);
  * requires the first instruction after the `svc` to *write* x0 without reading
    it, i.e. the caller provably ignores the result;
  * requires the number of surviving sites per syscall to equal the expected
    count, so a different glibc build fails loudly instead of quietly producing
    an image with the wrong instructions removed.
"""
import re
import struct
import sys

SVC = 0xD4000001
NOP = 0xD503201F

# Accelerators we neutralise: the caller provably ignores the result, so a `nop`
# leaves the program in exactly the state it expects when the kernel rejects the
# call.
PATCHABLE = {99: "set_robust_list"}
# Accelerators we only *locate*: their callers inspect the return value, so a
# `nop` would be wrong (it would hand them a stale x0). These are reported so a
# change in the image is visible, and the runtime keeps glibc from trying them
# with its own documented switch (glibc.pthread.rseq=0).
OBSERVED = {293: "rseq"}
ACCELERATORS = {**PATCHABLE, **OBSERVED}

# Instruction forms with Rd = the register we ask for. All are "write only".
def movz_imm(insn, rd):
    """mov{x,w} Rd, #imm  ->  imm or None"""
    if insn & 0xFFE0001F == 0xD2800000 | rd:          # movz x
        return (insn >> 5) & 0xFFFF, True
    if insn & 0xFFE0001F == 0x52800000 | rd:          # movz w
        return (insn >> 5) & 0xFFFF, True
    return None, False

def writes_x0_only(insn):
    """True if this instruction writes x0/w0 and cannot depend on its old value."""
    rd = insn & 0x1F
    if rd != 0:
        return False
    # movz/movn/movk w0|x0
    if insn & 0x7F800000 in (0x52800000, 0x12800000, 0x72800000, 0xD2800000,
                             0x92800000, 0xF2800000):
        return True
    # add/sub (immediate) x0, xN, #imm  /  add/sub x0, xN, xM  (no shift of x0 in)
    if insn & 0x7F000000 in (0x11000000, 0x51000000):
        return (insn >> 5) & 0x1F != 0
    if insn & 0x7FE00000 in (0x0B000000, 0x4B000000):
        return ((insn >> 5) & 0x1F) != 0 and ((insn >> 16) & 0x1F) != 0
    # ldr/ldur into x0/w0 from a base that is not x0
    if insn & 0x3B000000 == 0x39000000:                # ldr (immediate, unsigned)
        return (insn >> 5) & 0x1F != 0
    if insn & 0x3B200C00 == 0x38000000:                # ldur / ldr (immediate, signed)
        return (insn >> 5) & 0x1F != 0
    # mov w0, wN  (orr w0, wzr, wN)  /  mov x0, xN
    if insn & 0x7FE0FFE0 in (0x2A0003E0, 0xAA0003E0):
        return (insn >> 16) & 0x1F != 0
    return False

def is_branch(insn):
    return (insn & 0x7C000000 == 0x14000000      # b/bl
            or insn & 0xFE000000 == 0x54000000   # b.cond
            or insn & 0x7E000000 == 0x34000000   # cbz/cbnz
            or insn & 0x7E000000 in (0x36000000, 0x36000000)
            or insn & 0xFFFFFC1F in (0xD61F0000, 0xD63F0000, 0xD65F0000, 0xD67F0000)
            or insn in (0xD65F03C0,))            # ret

class Elf:
    def __init__(self, path):
        self.data = open(path, "rb").read()
        d = self.data
        assert d[:4] == b"\x7fELF" and d[4] == 2 and d[18:20] == b"\xb7\x00", \
            "not an aarch64 ELF64"
        self.e_phoff = struct.unpack_from("<Q", d, 0x20)[0]
        self.e_shoff = struct.unpack_from("<Q", d, 0x28)[0]
        shentsize, phnum = struct.unpack_from("<HH", d, 0x36)
        shnum = struct.unpack_from("<H", d, 0x3C)[0]
        self.phdrs = []
        for i in range(phnum):
            o = self.e_phoff + i * 56
            t, f = struct.unpack_from("<II", d, o)
            off, va, _, fsz, msz, al = struct.unpack_from("<QQQQQQ", d, o + 8)
            self.phdrs.append((t, f, off, va, fsz, msz, al))

    def vaddr_to_off(self, vaddr):
        for t, f, off, va, fsz, msz, al in self.phdrs:
            if t == 1 and va <= vaddr < va + fsz:
                return off + (vaddr - va)
        raise SystemExit(f"{vaddr:#x} is not file-backed")

    def exec_ranges(self):
        out = []
        for t, f, off, va, fsz, msz, al in self.phdrs:
            if t == 1 and f & 1:
                out.append((va, va + fsz))
        return out

    def glibc_version(self):
        """Highest GLIBC_2.x in .gnu.version_d, e.g. '2.39'."""
        # .gnu.version_d: find it through the shdr table (present in glibc's own
        # .so files; they keep section headers).
        shoff = struct.unpack_from("<Q", self.data, 0x28)[0]
        shnum = struct.unpack_from("<H", self.data, 0x3C)[0]
        secs = []
        for i in range(shnum):
            o = shoff + i * 64
            name, typ = struct.unpack_from("<II", self.data, o)
            addr, off, size = struct.unpack_from("<QQQ", self.data, o + 16)
            link = struct.unpack_from("<I", self.data, o + 40)[0]
            entsize = struct.unpack_from("<Q", self.data, o + 56)[0]
            secs.append((typ, addr, off, size, link, entsize))
        versym = [s for s in secs if s[0] == 0x6FFFFFFD]       # SHT_GNU_verdef
        if not versym:
            return None
        typ, addr, off, size, link, entsize = versym[0]
        strtab = secs[link]
        base = strtab[2]
        best = None
        p = 0
        while p < size:
            vd_version, vd_flags, vd_ndx, vd_cnt, vd_hash, vd_aux, vd_next = \
                struct.unpack_from("<HHHHIII", self.data, off + p)
            aux = p + vd_aux
            name_off = struct.unpack_from("<I", self.data, off + aux)[0]
            end = self.data.index(b"\0", base + name_off)
            name = self.data[base + name_off:end].decode()
            if name.startswith("GLIBC_") and not (vd_flags & 1):
                ver = name.split("_", 1)[1]
                if re.fullmatch(r"[0-9]+\.[0-9]+", ver):
                    if best is None or [int(x) for x in ver.split(".")] > \
                            [int(x) for x in best.split(".")]:
                        best = ver
            if vd_next == 0:
                break
            p += vd_next
        return best


def scan(elf):
    """Return [(vaddr, syscall, nr_name, prev_insns, next_insn)] for accelerator sites."""
    d = elf.data
    sites = []
    for start, end in elf.exec_ranges():
        off = elf.vaddr_to_off(start)
        vaddr = start
        while vaddr + 4 <= end:
            insn = struct.unpack_from("<I", d, off + (vaddr - start))[0]
            if insn == SVC:
                # walk back for `mov x8, #NR`
                nr = None
                back = []
                for k in range(1, 9):
                    prev_off = off + (vaddr - start) - 4 * k
                    if prev_off < 0:
                        break
                    prev = struct.unpack_from("<I", d, prev_off)[0]
                    if prev == SVC:
                        break
                    imm, ok = movz_imm(prev, 8)
                    if ok:
                        nr = imm
                        break
                    if is_branch(prev):
                        break
                    back.append(prev)
                if nr is not None and nr in ACCELERATORS:
                    nxt_off = off + (vaddr - start) + 4
                    nxt = struct.unpack_from("<I", d, nxt_off)[0] if nxt_off + 4 <= len(d) else 0
                    sites.append((vaddr, nr, ACCELERATORS[nr], back, nxt))
            vaddr += 4
    return sites


def fmt(insn):
    return f"{insn:#010x}"


def report(elf, path):
    ver = elf.glibc_version()
    print(f"{path}: glibc version from .gnu.version_d = {ver}")
    sites = scan(elf)
    for vaddr, nr, name, back, nxt in sites:
        ignored = writes_x0_only(nxt)
        print(f"  {vaddr:#08x}  {name}({nr})  "
              f"gap={len(back)}  next={fmt(nxt)}  "
              f"{'result ignored' if ignored else 'RESULT USED - do not nop'}")
        for b in reversed(back):
            print(f"          before {fmt(b)}")
    return ver, sites


def main(argv):
    mode = argv[0]
    path = argv[1]
    expect = {}
    args = argv[2:]
    out = None
    if mode == "patch":
        out = args[0]
        args = args[1:]
    want_version = None
    for i, a in enumerate(args):
        if a == "--expect":
            name, count = args[i + 1].split("=")
            expect[name] = int(count)
        elif a == "--version":
            want_version = args[i + 1]
    elf = Elf(path)
    ver, sites = report(elf, path)
    if want_version is not None and ver != want_version:
        print(f"REFUSING: this image says it is glibc {ver}, this build expects "
              f"{want_version} - the image shape (and therefore the sites) is "
              f"not the one checked")
        return 1
    bad = [s for s in sites if s[1] in PATCHABLE and not writes_x0_only(s[4])]
    if bad:
        print(f"REFUSING: {len(bad)} patchable site(s) use the syscall's return "
              f"value; a nop there would change behaviour")
        return 1
    if expect:
        for name, count in expect.items():
            got = sum(1 for s in sites if s[2] == name)
            if got != count:
                print(f"REFUSING: {name}: found {got} site(s), expected {count} - "
                      f"this is not the glibc build this script was checked against")
                return 1
    if mode == "patch":
        d = bytearray(elf.data)
        for vaddr, nr, name, back, nxt in sites:
            if nr not in PATCHABLE:
                continue
            o = elf.vaddr_to_off(vaddr)
            d[o:o + 4] = struct.pack("<I", NOP)
        open(out, "wb").write(d)
        n = sum(1 for s in sites if s[1] in PATCHABLE)
        print(f"patched {n} site(s) -> {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
