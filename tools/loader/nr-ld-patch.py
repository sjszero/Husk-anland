#!/usr/bin/env python3
"""Repoint the guest ld.so entry at the LinuxHub SIGSYS entry stub.

Adds no sections, no relocations and no new segments: the stub is written into
the tail of the loader's existing RX PT_LOAD (past the old p_filesz, inside an
already-mapped page), p_filesz/p_memsz are grown to cover it, and e_entry is
pointed at it. Deterministic and repeatable from the pristine loader.
"""
import struct, sys

STUB_ALIGN = 8
EXPECT_E_ENTRY = 0x1AA40
BRANCH_AT = 0x1AA40          # ld.so _start; a `nop` sits there
BRANCH_TO = 0x26730          # our stub
NOP = bytes.fromhex("1f2003d5")


def main(stub_path, src, dst):
    stub = open(stub_path, "rb").read()
    d = bytearray(open(src, "rb").read())
    assert d[:4] == b"\x7fELF" and d[4] == 2 and d[18:20] == b"\xb7\x00", "not aarch64 ELF64"
    e_phoff = struct.unpack_from("<Q", d, 0x20)[0]
    e_phentsize, e_phnum = struct.unpack_from("<HH", d, 0x36)
    assert e_phentsize == 56 and e_phnum >= 1

    # first PT_LOAD must be the RX one that starts at file offset 0
    p_type, p_flags = struct.unpack_from("<II", d, e_phoff)
    p_off, p_vaddr, _, p_filesz, p_memsz, p_align = struct.unpack_from("<QQQQQQ", d, e_phoff + 8)
    assert p_type == 1 and p_flags & 7 == 5, "first segment is not RX PT_LOAD"
    assert p_off == 0 and p_vaddr == 0, "first segment not mapped at 0"
    assert p_filesz == p_memsz, "first segment already has bss"

    e_entry = struct.unpack_from("<Q", d, 0x18)[0]
    assert e_entry == EXPECT_E_ENTRY, f"unexpected loader entry {e_entry:#x}"

    stub_vaddr = (p_filesz + STUB_ALIGN - 1) // STUB_ALIGN * STUB_ALIGN
    assert stub_vaddr == 0x26730, f"stub vaddr {stub_vaddr:#x} != the one compiled into the stub"

    # nothing else may live in the gap we are about to map
    e_shoff = struct.unpack_from("<Q", d, 0x28)[0]
    e_shentsize, e_shnum = struct.unpack_from("<HH", d, 0x3A)
    for i in range(e_shnum):
        sh = e_shoff + i * e_shentsize
        s_off, s_size = struct.unpack_from("<QQ", d, sh + 24)
        if s_size and s_off < stub_vaddr + len(stub) and s_off + s_size > p_filesz + 1:
            raise SystemExit(f"section {i} overlaps the stub area")

    end = stub_vaddr + len(stub)
    assert end <= 0x27000, "stub leaves the page that is already mapped"

    # e_entry stays 0x1aa40: replace the leading `nop` of _start with `b stub`
    assert d[BRANCH_AT:BRANCH_AT + 4] == NOP, "expected a nop at _start"
    imm26 = (BRANCH_TO - BRANCH_AT) // 4
    assert -(1 << 25) <= imm26 < (1 << 25), "branch out of range"
    struct.pack_into("<I", d, BRANCH_AT, 0x14000000 | (imm26 & 0x3FFFFFF))

    d[stub_vaddr:end] = stub
    struct.pack_into("<QQ", d, e_phoff + 32, end, end)   # p_filesz, p_memsz
    # keep exec permission on the whole segment for the new tail (already R E)
    open(dst, "wb").write(d)
    print(f"stub {len(stub)} bytes @ {stub_vaddr:#x}; b at _start {BRANCH_AT:#x} -> {BRANCH_TO:#x}; e_entry kept {e_entry:#x}")
    print(f"p_filesz {p_filesz:#x} -> {end:#x}, file {len(d)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:4])
