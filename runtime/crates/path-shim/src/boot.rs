//! Bootstrap for the loader-embedded path shim.
//!
//! This module runs inside the guest loader before anything else of ours is
//! usable, so it must reference **no external symbol at all**: every external
//! call it makes would go through a PLT slot that nobody has resolved yet.
//! Only volatile reads/writes, pointer arithmetic and one raw `mprotect`.
//!
//! What it does, in order:
//!   0. add the load bias to its own R_AARCH64_RELATIVE targets - the image was
//!      linked at a fixed address but mapped at a random one;
//!   1. for every entry of the build-time extern table, look the name up in the
//!      loaded objects (or in this object, if the name is ours) and write the
//!      real address into our own GOT slot;
//!   2. for every object in the link map, walk its relocations and point the GOT
//!      slots of the interposed names at our functions.
//!
//! Step 2 is what replaces LD_PRELOAD: symbol interposition normally comes from
//! the preload being earlier in the global scope, but an embedded object has no
//! scope of its own, so we do it by hand instead.

const DT_NULL: i64 = 0;
const DT_PLTRELSZ: i64 = 2;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_JMPREL: i64 = 23;
const DT_STRSZ: i64 = 10;
const DT_GNU_HASH: i64 = 0x6ffffef5;

const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;

const STT_GNU_IFUNC: u8 = 10;

const SYS_MPROTECT: usize = 226;
const PROT_READ: usize = 1;
const PROT_WRITE: usize = 2;

// glibc 2.39 struct link_map, 64-bit. Only the first four fields matter here.
const L_ADDR: usize = 0;
const L_NAME: usize = 8;
const L_LD: usize = 16;
const L_NEXT: usize = 24;

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const MAX_LOAD_RANGES: usize = 16;

#[derive(Clone, Copy)]
struct MapRange {
    start: u64,
    end: u64,
    flags: u32,
}

#[derive(Clone, Copy)]
struct ObjMap {
    ranges: [MapRange; MAX_LOAD_RANGES],
    count: usize,
}

// Boot is single-threaded and every helper only fills/reads this scratch map.
// Keeping it in BSS avoids compiler-generated stack memcpy/memset before the
// shim's own imports are resolved.
static mut MAP_SCRATCH: ObjMap = ObjMap {
    ranges: [MapRange {
        start: 0,
        end: 0,
        flags: 0,
    }; MAX_LOAD_RANGES],
    count: 0,
};

impl ObjMap {
    fn contains(&self, addr: usize, len: usize) -> bool {
        let start = addr as u64;
        let Some(end) = start.checked_add(len as u64) else {
            return false;
        };
        let mut i = 0;
        while i < self.count {
            let r = self.ranges[i];
            if start >= r.start && end <= r.end {
                return true;
            }
            i += 1;
        }
        false
    }

    fn exec_contains(&self, addr: usize, len: usize) -> bool {
        let start = addr as u64;
        let Some(end) = start.checked_add(len as u64) else {
            return false;
        };
        let mut i = 0;
        while i < self.count {
            let r = self.ranges[i];
            if r.flags & PF_X != 0 && start >= r.start && end <= r.end {
                return true;
            }
            i += 1;
        }
        false
    }
}

#[inline]
unsafe fn rd8(p: usize) -> u64 {
    core::ptr::read_volatile(p as *const u64)
}
#[inline]
unsafe fn rd4(p: usize) -> u32 {
    core::ptr::read_volatile(p as *const u32)
}
#[inline]
unsafe fn rd2(p: usize) -> u16 {
    core::ptr::read_volatile(p as *const u16)
}
#[inline]
unsafe fn rd1(p: usize) -> u8 {
    core::ptr::read_volatile(p as *const u8)
}
#[inline]
unsafe fn wr8(p: usize, v: u64) {
    core::ptr::write_volatile(p as *mut u64, v)
}

unsafe fn sys_mprotect(addr: usize, len: usize, prot: usize) -> isize {
    let ret: isize;
    core::arch::asm!("svc 0", in("x8") SYS_MPROTECT, in("x0") addr, in("x1") len,
        in("x2") prot, lateout("x0") ret, options(nostack));
    ret
}

const SYS_WRITE: usize = 64;

/// Raw write to fd 2. No formatting machinery: this runs before anything is
/// safe to call, and a boot failure with no output is unresearchable.
unsafe fn say(parts: &[&[u8]]) {
    let mut i = 0;
    while i < parts.len() {
        let p = parts[i];
        let _: isize;
        core::arch::asm!("svc 0", in("x8") SYS_WRITE, in("x0") 2usize,
            in("x1") p.as_ptr() as usize, in("x2") p.len(),
            lateout("x0") _, options(nostack));
        i += 1;
    }
}

unsafe fn say_hex(mut v: usize) {
    let mut buf = [b'0'; 18];
    let mut n = 0;
    let mut tmp = [0u8; 16];
    if v == 0 {
        say(&[b"0"]);
        return;
    }
    while v != 0 {
        tmp[n] = b"0123456789abcdef"[v & 0xf];
        v >>= 4;
        n += 1;
    }
    buf[0] = b'0';
    buf[1] = b'x';
    let mut i = 0;
    while i < n {
        buf[2 + i] = tmp[n - 1 - i];
        i += 1;
    }
    say(&[&buf[..2 + n]]);
}

unsafe fn mem_eq(a: usize, b: usize, n: usize) -> bool {
    let mut i = 0;
    while i < n {
        if rd1(a + i) != rd1(b + i) {
            return false;
        }
        i += 1;
    }
    true
}

unsafe fn map_from_phdr(phdr: usize, phnum: usize, l_addr: u64, map: &mut ObjMap) -> bool {
    map.count = 0;
    if phdr == 0 || phnum == 0 || phnum > 64 {
        return false;
    }
    let mut i = 0usize;
    while i < phnum {
        let off = match i.checked_mul(56) {
            Some(v) => v,
            None => return false,
        };
        let p = match phdr.checked_add(off) {
            Some(v) => v,
            None => return false,
        };
        if debug2_on() {
            say(&[b"nr-boot: phdr p="]);
            say_hex(p);
            say(&[b"\n"]);
        }
        let typ = rd4(p);
        let flags = rd4(p + 4);
        let vaddr = rd8(p + 16);
        let memsz = rd8(p + 40);
        if debug2_on() {
            say(&[b"nr-boot: phdr typ="]);
            say_hex(typ as usize);
            say(&[b" vaddr="]);
            say_hex(vaddr as usize);
            say(&[b" memsz="]);
            say_hex(memsz as usize);
            say(&[b"\n"]);
        }
        if typ == PT_LOAD && memsz != 0 {
            if map.count >= MAX_LOAD_RANGES {
                return false;
            }
            let start = match l_addr.checked_add(vaddr) {
                Some(v) => v,
                None => return false,
            };
            let end = match start.checked_add(memsz) {
                Some(v) => v,
                None => return false,
            };
            if end <= start {
                return false;
            }
            map.ranges[map.count] = MapRange { start, end, flags };
            map.count += 1;
        }
        i += 1;
    }
    map.count != 0
}

/// Build the load ranges for one link-map object. ET_DYN objects expose their
/// ELF header at l_addr. The main ET_EXEC object has l_addr == 0, so its header
/// may be unmapped; use the kernel-provided AT_PHDR/AT_PHNUM instead.
unsafe fn object_map(l_addr: u64, l_ld: usize, map: &mut ObjMap) -> bool {
    map.count = 0;
    // On arm64 Android, every mapped ELF base is either 0 (ET_EXEC) or a real
    // high userspace address. Small non-zero "addresses" are malformed link-map
    // data; refuse to dereference them and fail closed instead of faulting.
    const MIN_USER_MAPPING: u64 = 0x1_0000_0000;
    if l_addr != 0 && l_addr < MIN_USER_MAPPING {
        return false;
    }
    // ET_EXEC keeps link-time virtual addresses (for example its dynamic table
    // at 0xa1fdb0) with l_addr == 0. Only a relocated ET_DYN object must have a
    // high mapped l_ld; the PT_LOAD range check below validates ET_EXEC.
    if l_addr != 0 && l_ld != 0 && (l_ld as u64) < MIN_USER_MAPPING {
        return false;
    }
    if debug2_on() {
        say(&[b"nr-boot: objmap magic\n"]);
    }
    if l_addr != 0 && rd4(l_addr as usize) == 0x464c_457f {
        let base = l_addr as usize;
        if debug2_on() {
            say(&[b"nr-boot: objmap header\n"]);
        }
        if rd1(base + 4) != 2 || rd1(base + 5) != 1 {
            return false;
        }
        let phoff = rd8(base + 32);
        let phentsize = rd2(base + 54) as usize;
        let phnum = rd2(base + 56) as usize;
        if debug2_on() {
            say(&[b"nr-boot: objmap phoff="]);
            say_hex(phoff as usize);
            say(&[b" entsize="]);
            say_hex(phentsize);
            say(&[b" phnum="]);
            say_hex(phnum);
            say(&[b"\n"]);
        }
        if phentsize != 56 || phnum == 0 || phnum > 64 {
            return false;
        }
        let phoff = match usize::try_from(phoff) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let phdr = match base.checked_add(phoff) {
            Some(v) => v,
            None => return false,
        };
        if debug2_on() {
            say(&[b"nr-boot: objmap computed phdr="]);
            say_hex(phdr);
            say(&[b"\n"]);
        }
        let ok = map_from_phdr(phdr, phnum, l_addr, map);
        if debug2_on() {
            say(&[b"nr-boot: objmap parsed="]);
            say_hex(if ok { 1 } else { 0 });
            say(&[b"\n"]);
        }
        return ok;
    }
    let main_phdr = MAIN_PHDR;
    let main_phnum = MAIN_PHNUM;
    if l_addr == 0 && main_phdr != 0 && main_phnum != 0 {
        if map_from_phdr(main_phdr, main_phnum, 0, map) && map.contains(l_ld, 16) {
            return true;
        }
    }
    false
}

unsafe fn norm_dyn(v: u64, l_addr: u64, map: &ObjMap) -> usize {
    let p = v as usize;
    if p != 0 && map.contains(p, 1) {
        return p;
    }
    if l_addr != 0 {
        let q = p.wrapping_add(l_addr as usize);
        if map.contains(q, 1) {
            return q;
        }
    }
    0
}

/// Look one value up in an object's PT_DYNAMIC. Every read is checked against
/// the object's PT_LOAD ranges; a malformed dynamic table cannot walk into an
/// unrelated mapping.
unsafe fn dyn_val(l_ld: usize, tag: i64, map: &ObjMap) -> u64 {
    if l_ld == 0 || !map.contains(l_ld, 16) {
        return 0;
    }
    let mut p = l_ld;
    let mut guard = 0;
    while guard < 1024 {
        if !map.contains(p, 16) {
            return 0;
        }
        let t = rd8(p) as i64;
        if t == DT_NULL {
            return 0;
        }
        if t == tag {
            return rd8(p + 8);
        }
        p += 16;
        guard += 1;
    }
    0
}

fn gnu_hash(name: usize, len: usize) -> u32 {
    let mut h: u32 = 5381;
    let mut i = 0;
    unsafe {
        while i < len {
            h = h.wrapping_mul(33).wrapping_add(rd1(name + i) as u32);
            i += 1;
        }
    }
    h
}

/// Resolve `name` in one object. Returns an absolute address or 0.
unsafe fn find_sym(l_addr: u64, l_ld: usize, name: usize, len: usize) -> u64 {
    let map_ptr = core::ptr::addr_of_mut!(MAP_SCRATCH);
    if !object_map(l_addr, l_ld, &mut *map_ptr) {
        return 0;
    }
    let map = &*map_ptr;
    if debug2_on() {
        say(&[b"nr-boot: map ranges="]);
        say_hex(map.count);
        say(&[b" first="]);
        say_hex(map.ranges[0].start as usize);
        say(&[b" last="]);
        say_hex(map.ranges[map.count - 1].end as usize);
        say(&[b"\n"]);
    }
    if len == 0 || len > 255 {
        return 0;
    }

    let strtab = norm_dyn(dyn_val(l_ld, DT_STRTAB, &map), l_addr, &map);
    let symtab = norm_dyn(dyn_val(l_ld, DT_SYMTAB, &map), l_addr, &map);
    let hash = norm_dyn(dyn_val(l_ld, DT_GNU_HASH, &map), l_addr, &map);
    let strsz = dyn_val(l_ld, DT_STRSZ, &map) as usize;
    if debug2_on() {
        say(&[b"nr-boot: tables str="]);
        say_hex(strtab);
        say(&[b" sym="]);
        say_hex(symtab);
        say(&[b" hash="]);
        say_hex(hash);
        say(&[b" strsz="]);
        say_hex(strsz);
        say(&[b"\n"]);
    }
    if strtab == 0
        || symtab == 0
        || hash == 0
        || strsz == 0
        || !map.contains(strtab, strsz)
        || !map.contains(hash, 16)
    {
        return 0;
    }

    let nbuckets = rd4(hash) as usize;
    let symndx = rd4(hash + 4) as usize;
    let maskwords = rd4(hash + 8) as usize;
    if nbuckets == 0 || nbuckets > (1 << 20) || maskwords > (1 << 20) || symndx > (1 << 24) {
        return 0;
    }
    let bloom_size = match maskwords.checked_mul(8) {
        Some(v) => v,
        None => return 0,
    };
    let bucket_size = match nbuckets.checked_mul(4) {
        Some(v) => v,
        None => return 0,
    };
    let buckets = match hash.checked_add(16).and_then(|v| v.checked_add(bloom_size)) {
        Some(v) => v,
        None => return 0,
    };
    let chain = match buckets.checked_add(bucket_size) {
        Some(v) => v,
        None => return 0,
    };
    if debug2_on() {
        say(&[b"nr-boot: hash buckets="]);
        say_hex(buckets);
        say(&[b" chain="]);
        say_hex(chain);
        say(&[b"\n"]);
    }
    if !map.contains(buckets, bucket_size) || !map.contains(chain, 4) {
        return 0;
    }

    let h = gnu_hash(name, len);
    let bucket = match (h as usize % nbuckets).checked_mul(4) {
        Some(v) => v,
        None => return 0,
    };
    let mut idx = rd4(buckets + bucket) as usize;
    if debug2_on() {
        say(&[b"nr-boot: gnu h="]);
        say_hex(h as usize);
        say(&[b" bucket="]);
        say_hex((h as usize % nbuckets) * 4);
        say(&[b" idx="]);
        say_hex(idx);
        say(&[b"\n"]);
    }
    if idx < symndx {
        return 0;
    }
    let mut guard = 0usize;
    loop {
        if guard >= (1 << 20) {
            return 0;
        }
        let chain_off = match (idx - symndx).checked_mul(4) {
            Some(v) => v,
            None => return 0,
        };
        let cp = match chain.checked_add(chain_off) {
            Some(v) => v,
            None => return 0,
        };
        if debug2_on() {
            say(&[b"nr-boot: chain cp="]);
            say_hex(cp);
            say(&[b" idx="]);
            say_hex(idx);
            say(&[b"\n"]);
        }
        if !map.contains(cp, 4) {
            return 0;
        }
        let c = rd4(cp);
        if debug2_on() {
            say(&[b"nr-boot: chain c="]);
            say_hex(c as usize);
            say(&[b"\n"]);
        }
        if (c | 1) == (h | 1) {
            let sym_off = match idx.checked_mul(24) {
                Some(v) => v,
                None => return 0,
            };
            let sp = match symtab.checked_add(sym_off) {
                Some(v) => v,
                None => return 0,
            };
            if debug2_on() {
                say(&[b"nr-boot: sym sp="]);
                say_hex(sp);
                say(&[b"\n"]);
            }
            if !map.contains(sp, 24) {
                return 0;
            }
            let st_name = rd4(sp) as usize;
            if st_name >= strsz || len >= strsz - st_name {
                return 0;
            }
            let nm = strtab + st_name;
            if !map.contains(nm, len + 1) {
                return 0;
            }
            if rd1(nm + len) == 0 && mem_eq(nm, name, len) {
                let st_value = rd8(sp + 8);
                let st_info = rd1(sp + 4);
                let addr = match l_addr.checked_add(st_value) {
                    Some(v) => v,
                    None => return 0,
                };
                if !map.contains(addr as usize, 1) {
                    return 0;
                }
                // An IFUNC resolves to a resolver function; call it, exactly like
                // the loader does, otherwise callers get the resolver's address.
                if (st_info & 0xf) == STT_GNU_IFUNC {
                    if !map.exec_contains(addr as usize, 4) {
                        return 0;
                    }
                    let f: extern "C" fn() -> usize = core::mem::transmute(addr as usize);
                    return f() as u64;
                }
                return addr;
            }
        }
        if c & 1 != 0 {
            return 0;
        }
        idx = match idx.checked_add(1) {
            Some(v) => v,
            None => return 0,
        };
        guard += 1;
    }
}

/// Resolve `name` anywhere in the link map chain.
unsafe fn resolve(main_map: usize, name: usize, len: usize) -> u64 {
    let mut l = main_map;
    let mut guard = 0;
    while l != 0 && guard < 4096 {
        let l_addr = rd8(l + L_ADDR);
        let l_ld = rd8(l + L_LD) as usize;
        if debug2_on() {
            say(&[b"nr-boot: lookup obj="]);
            say_hex(guard);
            say(&[b" l_addr="]);
            say_hex(l_addr as usize);
            say(&[b" l_ld="]);
            say_hex(l_ld);
            say(&[b"\n"]);
        }
        let a = find_sym(l_addr, l_ld, name, len);
        if debug2_on() {
            say(&[b"nr-boot: lookup -> "]);
            say_hex(a as usize);
            say(&[b"\n"]);
        }
        if a != 0 {
            return a;
        }
        l = rd8(l + L_NEXT) as usize;
        guard += 1;
    }
    0
}

static mut PAGE: usize = 0;
/// The kernel's page size, read once from the auxiliary vector (AT_PAGESZ).
/// Zero when the auxv is missing, in which case we fall back to a probe. Never
/// assume 4096: a 16 KiB kernel would silently round every mprotect wrong.
static mut KERNEL_PAGE: usize = 0;

unsafe fn page_size() -> usize {
    if PAGE != 0 {
        return PAGE;
    }
    let k = KERNEL_PAGE;
    if k >= 4096 && k.is_power_of_two() {
        PAGE = k;
        return PAGE;
    }
    // Probe on our own BSS: the loader hook has already made that window
    // writable, so this only asks "is a 4K-aligned address a legal page start
    // here?". A 16K/64K page kernel answers EINVAL. If the probe address happens
    // to be aligned for both, 4K is assumed; every window we then mprotect comes
    // from a page-aligned object segment, so a wrong answer degrades to a failed
    // make_writable() rather than to corruption.
    let here = core::ptr::addr_of!(PAGE) as usize;
    let a4 = here & !0xfff;
    PAGE = if sys_mprotect(a4, 4096, PROT_READ | PROT_WRITE) == 0 {
        4096
    } else {
        16384
    };
    PAGE
}

unsafe fn make_writable(addr: usize, len: usize) -> bool {
    let ps = page_size();
    let start = addr & !(ps - 1);
    let end = (addr + len + ps - 1) & !(ps - 1);
    sys_mprotect(start, end - start, PROT_READ | PROT_WRITE) == 0
}

struct Tables;

impl Tables {
    #[inline]
    unsafe fn count(t: usize) -> usize {
        rd8(t) as usize
    }
    // entry layout, 24 bytes: { name_vaddr, name_len, payload }
    //   externs   payload = loader-relative address of our PLT slot
    //   interpose payload = loader-relative address of our function
    #[inline]
    unsafe fn entry(t: usize, i: usize) -> usize {
        t + 8 + i * 24
    }
    #[inline]
    unsafe fn slot(t: usize, i: usize) -> usize {
        rd8(Tables::entry(t, i) + 16) as usize
    }
    #[inline]
    unsafe fn name(t: usize, i: usize) -> (usize, usize) {
        let e = Tables::entry(t, i);
        (rd8(e) as usize, rd8(e + 8) as usize)
    }
}

/// Entry point called from the loader hook with `main_map` (the link_map the
/// loader would hand to _dl_init) and loader-relative table addresses.
/// Returns 0 on success, otherwise a step code so a failure is visible.
static mut BOOTED: u32 = 0;
static mut ENVP: usize = 0;
static mut MAIN_MAP: usize = 0;
static mut AT_RANDOM_WORDS: u64 = 0;

/// Per-process entropy the kernel already placed in memory (auxv AT_RANDOM).
/// Read here because the only other source, getrandom(2), is trapped by this
/// device's seccomp policy and kills the process with SIGSYS.
pub unsafe fn at_random() -> u64 {
    AT_RANDOM_WORDS
}

/// The environment vector the loader was started with (NULL before glibc's
/// start-up installs `environ`).
pub unsafe fn envp() -> usize {
    ENVP
}

static mut EXECFN: usize = 0;
static mut LOADER_BASE: usize = 0;
static mut LIBC_BASE: u64 = 0;
static mut MAIN_PHDR: usize = 0;
static mut MAIN_PHNUM: usize = 0;

pub unsafe fn loader_base() -> usize {
    LOADER_BASE
}
pub unsafe fn libc_base() -> u64 {
    LIBC_BASE
}

unsafe fn auxv_ptr(envp: usize) -> usize {
    if envp == 0 {
        return 0;
    }
    let mut p = envp;
    let mut i = 0;
    while i < 4096 && rd8(p) != 0 {
        p += 8;
        i += 1;
    }
    if i >= 4096 {
        return 0;
    }
    p + 8
}

unsafe fn auxv_val(envp: usize, tag: u64) -> u64 {
    let mut p = auxv_ptr(envp);
    if p == 0 {
        return 0;
    }
    let mut j = 0;
    while j < 64 {
        let k = rd8(p);
        if k == 0 {
            return 0;
        }
        if k == tag {
            return rd8(p + 8);
        }
        p += 16;
        j += 1;
    }
    0
}

unsafe fn read_at_random(envp: usize) -> u64 {
    let mut p = auxv_ptr(envp);
    if p == 0 {
        return 0;
    }
    let mut j = 0;
    while j < 64 {
        let k = rd8(p);
        if k == 0 {
            return 0;
        }
        if k == 25 {
            return rd8(p + 8);
        } // AT_RANDOM -> 16 random bytes
        p += 16;
        j += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nr_boot(
    base: usize,
    main_map: usize,
    ext: usize,
    interp: usize,
    envp: usize,
    relocs: usize,
) -> usize {
    // _dl_init is reached once per process, but be explicit about it: running the
    // GOT rewrite twice would be harmless, while a partial second run (e.g. after
    // a failed first one) would be confusing.
    if BOOTED != 0 {
        return 0;
    }
    BOOTED = 1;
    if base == 0 || main_map == 0 || ext == 0 || interp == 0 {
        say(&[b"nr-boot: invalid descriptor\n"]);
        return 1;
    }

    // 0. finish our own relocation. The build bakes "link-time address" into
    //    every R_AARCH64_RELATIVE target; the load bias is only known now, so
    //    it is added here before anything dereferences those pointers.
    if relocs != 0 {
        let n = rd8(relocs) as usize;
        let mut i = 0;
        while i < n {
            // same 24-byte entry layout as the other tables; only the payload
            // (the slot address) is used here
            let slot = base + rd8(relocs + 8 + i * 24 + 16) as usize;
            wr8(slot, rd8(slot).wrapping_add(base as u64));
            i += 1;
        }
    }

    MAIN_MAP = main_map;
    ENVP = envp;
    LOADER_BASE = base;
    if AT_RANDOM_WORDS == 0 {
        AT_RANDOM_WORDS = read_at_random(envp);
    }
    if EXECFN == 0 {
        EXECFN = auxv_val(envp, 31) as usize;
    } // AT_EXECFN
    if KERNEL_PAGE == 0 {
        KERNEL_PAGE = auxv_val(envp, 6) as usize;
    } // AT_PAGESZ
    if MAIN_PHDR == 0 {
        MAIN_PHDR = auxv_val(envp, 3) as usize; // AT_PHDR
        MAIN_PHNUM = auxv_val(envp, 5) as usize; // AT_PHNUM
    }
    if debug2_on() {
        say(&[b"nr-boot: base="]);
        say_hex(base);
        say(&[b" main_map="]);
        say_hex(main_map);
        say(&[b"\n"]);
        say(&[b"nr-boot: exec="]);
        if EXECFN != 0 {
            say(&[core::slice::from_raw_parts(
                EXECFN as *const u8,
                strlen(EXECFN),
            )]);
        }
        say(&[b"\n"]);
    }

    // 1. our own externals
    let n = Tables::count(ext);
    if n > 1024 {
        say(&[b"nr-boot: extern table too large\n"]);
        return 72;
    }
    let mut i = 0;
    while i < n {
        let (name, len) = Tables::name(ext, i);
        if debug2_on() {
            say(&[b"nr-boot: extern "]);
            say(&[core::slice::from_raw_parts((base + name) as *const u8, len)]);
            say(&[b"\n"]);
        }
        // A symbol this object also exports (a PLT call to one of our own
        // functions) binds to the object itself, exactly as it would if this
        // were a preloaded DSO: the first definition in scope wins.
        let addr = match self_lookup(base, interp, base + name, len) {
            0 => resolve(main_map, base + name, len),
            a => a,
        };
        if addr == 0 {
            say(&[b"nr-boot: unresolved import: "]);
            say(&[
                core::slice::from_raw_parts((base + name) as *const u8, len),
                b"\n",
            ]);
            return 10 + i;
        }
        let slot = base + Tables::slot(ext, i);
        if !make_writable(slot, 8) {
            return 40 + i;
        }
        wr8(slot, addr);
        i += 1;
    }

    // 2. interpose by rewriting the GOT of every loaded object
    let count = Tables::count(interp);
    INTERP = interp;
    INTERP_N = count;
    if count > 4096 {
        say(&[b"nr-boot: interpose table too large\n"]);
        return 73;
    }
    let mut l = main_map;
    let mut guard = 0;
    let mut objects = 0;
    let mut patched = 0;
    let mut patch_failures = 0;
    let mut slot_targets = 0;
    while l != 0 && guard < 4096 {
        let l_addr = rd8(l + L_ADDR);
        let l_ld = rd8(l + L_LD) as usize;
        let (n_here, failed_here) = patch_object(l_addr, l_ld, base, interp, count);
        if debug2_on() {
            let nm = rd8(l + L_NAME) as usize;
            say(&[b"nr-boot: patch @"]);
            say_hex(l_addr as usize);
            say(&[b" "]);
            if nm != 0 {
                say(&[core::slice::from_raw_parts(nm as *const u8, strlen(nm))]);
            } else {
                say(&[b"?"]);
            }
            say(&[b"="]);
            say_hex(n_here);
            say(&[b" failed="]);
            say_hex(failed_here);
            say(&[b"\n"]);
        }
        patched += n_here;
        patch_failures += failed_here;
        slot_targets += fill_slot_page(l_addr, base, interp);
        objects += 1;
        l = rd8(l + L_NEXT) as usize;
        guard += 1;
    }
    if l != 0 {
        say(&[b"nr-boot: link_map chain truncated\n"]);
        return 70;
    }
    if patch_failures != 0 {
        say(&[b"nr-boot: GOT patch incomplete: "]);
        say_hex(patch_failures);
        say(&[b"\n"]);
        return 71;
    }

    // 2b. patch the *definition* of every path entry point inside libc.so.6.
    //     Step 2 only rewrites GOT slots, which binds calls that go through the
    //     PLT -- i.e. calls made by the program and by other DSOs. c library
    //     internals (_IO_file_open, nss_files' fopen, opendir, realpath, the
    //     locale loader) call these functions directly, with no GOT slot in
    //     between, so on this kernel they reached the *host* "/etc/passwd" and
    //     failed: `getpwuid()` returned NULL ("I have no name!"), `groups` could
    //     not name a single gid, even though the file is right there in the
    //     rootfs and `cat` reads it fine.
    //
    //     Only wrappers that never call the original symbol are listed: ours are
    //     built on raw syscalls and runtime-core, so no body patch can recurse
    //     into itself. exec/fork/spawn, the signal wrappers and dlopen stay out.
    //
    // OFF by default: on this kernel+policy the second mprotect (back to r-x)
    // fails with EACCES on an app domain, so a patched page loses execute and
    // the next instruction fetch there faults. The same redirects therefore have
    // to be baked into libc at build time, with the target address in a writable
    // slot; see docs/design/libc-interpose.md. Kept behind NR_BODY_PATCH=1
    // because the bisect knobs (NR_BODY_N / NR_BODY_ONLY) live here.
    let libc_bodies = if body_patch_on() {
        patch_libc_bodies(base, interp, main_map)
    } else {
        0
    };

    // 3. the seccomp trap catcher. This device's policy answers some optional
    //    syscalls with SIGSYS, and a handler can only be installed by us: the
    //    embedded object has no .init_array of its own that the loader would run.
    crate::install_sigsys();

    // 4. start at the guest root, not at the host directory the app launcher
    //    picked (see set_guest_cwd).
    crate::set_guest_cwd();

    if debug_on() {
        say(&[b"nr-boot: imports="]);
        say_hex(n);
        say(&[b" objects="]);
        say_hex(objects);
        say(&[b" slots="]);
        say_hex(patched);
        say(&[b" bodies="]);
        say_hex(libc_bodies);
        say(&[b" slot_targets="]);
        say_hex(slot_targets);
        say(&[b"\n"]);
    }
    0
}

/// Diagnostics are opt-in through the environment (`NR_BOOT_DEBUG=1`) so the
/// default run stays silent. The environment comes from the loader (_dl_init's
/// fourth argument), not from `environ`: glibc has not run __libc_start_main
/// yet at this point, so `environ` is still NULL.
unsafe fn strlen(p: usize) -> usize {
    let mut n = 0;
    while n < 4096 && rd1(p + n) != 0 {
        n += 1;
    }
    n
}

unsafe fn debug_var(want: &[u8]) -> bool {
    let env = ENVP;
    if env == 0 {
        return false;
    }
    let mut i = 0;
    while i < 512 {
        let e = rd8(env + i * 8) as usize;
        if e == 0 {
            return false;
        }
        if mem_eq(e, want.as_ptr() as usize, want.len()) {
            return true;
        }
        i += 1;
    }
    false
}

unsafe fn debug2_on() -> bool {
    debug_var(b"NR_BOOT_DEBUG2=1")
}

unsafe fn debug_on() -> bool {
    let env = ENVP;
    if env == 0 {
        return false;
    }
    let want = b"NR_BOOT_DEBUG=1";
    let mut i = 0;
    while i < 512 {
        let e = rd8(env + i * 8) as usize;
        if e == 0 {
            return false;
        }
        if mem_eq(e, want.as_ptr() as usize, want.len()) {
            return true;
        }
        i += 1;
    }
    false
}

/// Look a name up in this object's own interpose table. Returns an absolute
/// address, or 0 if the name is not ours.
unsafe fn self_lookup(base: usize, interp: usize, name: usize, len: usize) -> u64 {
    let count = Tables::count(interp);
    let mut i = 0;
    while i < count {
        let (nm, nl) = Tables::name(interp, i);
        if nl == len && mem_eq(base + nm, name, len) {
            return (base + Tables::slot(interp, i)) as u64;
        }
        i += 1;
    }
    0
}

/// Path entry points whose libc.so.6 definition is redirected into the shim.
/// Every one of these has a standalone implementation in the shim (raw syscalls
/// plus runtime-core), so redirecting libc's internal callers cannot recurse.
static BODY_PATCH: [&[u8]; 93] = [
    b"open",
    b"open64",
    b"openat",
    b"openat64",
    b"__open_2",
    b"__open64_2",
    b"__openat_2",
    b"__openat64_2",
    b"stat",
    b"stat64",
    b"lstat",
    b"lstat64",
    b"fstatat",
    b"fstatat64",
    b"statx",
    b"statfs",
    b"access",
    b"faccessat",
    b"eaccess",
    b"euidaccess",
    b"mkdir",
    b"mkdirat",
    b"rmdir",
    b"unlink",
    b"unlinkat",
    b"rename",
    b"renameat",
    b"renameat2",
    b"symlink",
    b"symlinkat",
    b"link",
    b"linkat",
    b"chmod",
    b"fchmodat",
    b"chown",
    b"lchown",
    b"fchownat",
    b"truncate",
    b"utimensat",
    b"utimes",
    b"readlink",
    b"readlinkat",
    b"__readlink_chk",
    b"__readlinkat_chk",
    b"chdir",
    b"fchdir",
    b"getcwd",
    b"opendir",
    b"fopen",
    b"fopen64",
    b"creat",
    b"creat64",
    b"realpath",
    b"__realpath_chk",
    b"canonicalize_file_name",
    b"mkstemp",
    b"mkstemps",
    b"mkostemp",
    b"mkostemps",
    b"mkdtemp",
    b"mount",
    b"umount2",
    b"getxattr",
    b"lgetxattr",
    b"mq_open",
    b"__mq_open_2",
    b"mq_close",
    b"mq_unlink",
    b"mq_send",
    b"mq_receive",
    b"mq_timedsend",
    b"mq_timedreceive",
    b"mq_getattr",
    b"mq_setattr",
    b"mq_notify",
    b"shmget",
    b"shmat",
    b"shmdt",
    b"shmctl",
    b"semget",
    b"semctl",
    b"semop",
    b"semtimedop",
    b"msgget",
    b"msgsnd",
    b"msgrcv",
    b"msgctl",
    b"socket",
    b"getsockname",
    b"send",
    b"sendto",
    b"sendmsg",
    b"recvmsg",
];

/// `NR_BODY_N=<n>` patches only the first n names. Used to bisect which body
/// patch breaks a program; unset means "all".
unsafe fn body_limit() -> usize {
    let key = b"NR_BODY_N=";
    let env = ENVP;
    if env == 0 {
        return BODY_PATCH.len();
    }
    let mut i = 0;
    while i < 512 {
        let e = rd8(env + i * 8) as usize;
        if e == 0 {
            return BODY_PATCH.len();
        }
        if mem_eq(e, key.as_ptr() as usize, key.len()) {
            let mut v = 0usize;
            let mut j = key.len();
            while rd1(e + j) >= b'0' && rd1(e + j) <= b'9' {
                v = v * 10 + (rd1(e + j) - b'0') as usize;
                j += 1;
            }
            return v.min(BODY_PATCH.len());
        }
        i += 1;
    }
    BODY_PATCH.len()
}

unsafe fn body_patch_on() -> bool {
    debug_var(b"NR_BODY_PATCH=1")
}

/// `NR_BODY_ONLY=<name>` restricts the bisect knob to a single symbol.
unsafe fn body_only() -> Option<(usize, usize)> {
    let key = b"NR_BODY_ONLY=";
    let env = ENVP;
    if env == 0 {
        return None;
    }
    let mut i = 0;
    while i < 512 {
        let e = rd8(env + i * 8) as usize;
        if e == 0 {
            return None;
        }
        if mem_eq(e, key.as_ptr() as usize, key.len()) {
            let mut n = 0;
            while n < 64 && rd1(e + key.len() + n) != 0 {
                n += 1;
            }
            if n == 0 {
                return None;
            }
            return Some((e + key.len(), n));
        }
        i += 1;
    }
    None
}

unsafe fn in_body_list(nm: usize, len: usize) -> bool {
    if let Some((want, wlen)) = body_only() {
        return wlen == len && mem_eq(nm, want, len);
    }
    let mut i = 0;
    while i < BODY_PATCH.len() {
        let cand = BODY_PATCH[i];
        if cand.len() == len && mem_eq(nm, cand.as_ptr() as usize, len) {
            return true;
        }
        i += 1;
    }
    false
}

#[inline]
unsafe fn wr4(p: usize, v: u32) {
    core::ptr::write_volatile(p as *mut u32, v);
}

/// Overwrite the first 16 bytes of a function with
/// `ldr x16, #8 ; br x16 ; .quad target`. Callers still branch to the same
/// address, so every internal caller is redirected at once.
unsafe fn patch_body(at: usize, to: usize) -> bool {
    const PROT_EXEC: usize = 4;
    let ps = page_size();
    let start = at & !(ps - 1);
    let end = (at + 16 + ps - 1) & !(ps - 1);
    let r1 = sys_mprotect(start, end - start, PROT_READ | PROT_WRITE);
    if r1 != 0 {
        if debug2_on() {
            say(&[b"  body-rw-fail "]);
            say_hex(at);
            say(&[b" r="]);
            say_hex(r1 as usize);
            say(&[b"\n"]);
        }
        return false;
    }
    wr4(at, 0x5800_0050); // ldr x16, #8
    wr4(at + 4, 0xD61F_0200); // br x16
    wr8(at + 8, to as u64);
    flush_icache(at, 16);
    let r2 = sys_mprotect(start, end - start, PROT_READ | PROT_EXEC);
    if debug2_on() {
        say(&[b"  body-rx "]);
        say_hex(at);
        say(&[b" r="]);
        say_hex(r2 as usize);
        say(&[b"\n"]);
    }
    true
}

/// Data-cache clean + instruction-cache invalidate for the patched range.
/// There is no other thread yet (this runs inside _dl_init), so `ish` is enough.
unsafe fn flush_icache(at: usize, len: usize) {
    let ctr: u64;
    core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
    let dline = 4u64 << (ctr & 0xf);
    let iline = 4u64 << ((ctr >> 16) & 0xf);
    let end = (at + len) as u64;
    let mut p = at as u64 & !(dline - 1);
    while p < end {
        core::arch::asm!("dc cvau, {}", in(reg) p, options(nostack));
        p += dline;
    }
    core::arch::asm!("dsb ish", options(nostack));
    let mut p = at as u64 & !(iline - 1);
    while p < end {
        core::arch::asm!("ic ivau, {}", in(reg) p, options(nostack));
        p += iline;
    }
    core::arch::asm!("dsb ish", "isb", options(nostack));
}

/// `nr-tools libc-interpose` bakes a dispatcher into libc's own path entry
/// points: each one loads a target out of a table and jumps to it, falling back
/// to a copy of its original instructions while the entry is still zero. The
/// GOT rewrite above only binds calls that pass through a PLT, so this is what
/// covers the c library calling *itself* -- the resolver reading
/// `/etc/resolv.conf`, `_IO_file_open` reading `/etc/passwd`, the locale loader.
///
/// Returns how many entries were filled.
/// `NR_SLOTS_N=<n>` fills only the first n entries, so a bisect can point at
/// the one redirect that breaks a program. Unset means "all".
unsafe fn slots_limit() -> usize {
    let key = b"NR_SLOTS_N=";
    let env = ENVP;
    if env == 0 {
        return usize::MAX;
    }
    let mut i = 0;
    while i < 512 {
        let e = rd8(env + i * 8) as usize;
        if e == 0 {
            break;
        }
        if mem_eq(e, key.as_ptr() as usize, key.len()) {
            let mut v = 0usize;
            let mut j = key.len();
            while rd1(e + j) >= b'0' && rd1(e + j) <= b'9' {
                v = v * 10 + (rd1(e + j) - b'0') as usize;
                j += 1;
            }
            return v;
        }
        i += 1;
    }
    usize::MAX
}

unsafe fn fill_slot_page(l_addr: u64, base: usize, interp: usize) -> usize {
    const SLOT_MAGIC: [u8; 12] = *b"HUSK-SLOT-1\0";
    const SLOT_HDR: usize = 24;
    const SLOT_STRIDE: usize = 48;
    let eh = l_addr as usize;
    if eh == 0 {
        return 0;
    }
    let phoff = rd8(eh + 0x20) as usize;
    let phnum = rd2(eh + 0x38) as usize;
    if phnum == 0 || phnum > 64 {
        return 0;
    }
    let mut i = 0;
    while i < phnum {
        let p = eh + phoff + i * 56;
        if rd4(p) == PT_LOAD {
            let page = eh + rd8(p + 16) as usize;
            if mem_eq(page, SLOT_MAGIC.as_ptr() as usize, SLOT_MAGIC.len()) {
                let count = rd4(page + 12) as usize;
                let stride = rd4(page + 16) as usize;
                if stride != SLOT_STRIDE || count > 256 {
                    return 0;
                }
                let mut filled = 0;
                let limit = slots_limit();
                let mut k = 0;
                while k < count && k < limit {
                    let e = page + SLOT_HDR + k * stride;
                    let mut nl = 0;
                    while nl < 32 && rd1(e + nl) != 0 {
                        nl += 1;
                    }
                    if nl > 0 {
                        let target = self_lookup(base, interp, e, nl);
                        if target != 0 {
                            // The page is part of a writable PT_LOAD by
                            // construction, so no mprotect is involved.
                            wr8(e + 32, target);
                            filled += 1;
                        }
                    }
                    k += 1;
                }
                return filled;
            }
        }
        i += 1;
    }
    0
}

/// Walk the link map for libc.so.6 and redirect its path entry points.
unsafe fn patch_libc_bodies(base: usize, interp: usize, main_map: usize) -> usize {
    let mut l = main_map;
    let mut guard = 0;
    while l != 0 && guard < 4096 {
        let nm = rd8(l + L_NAME) as usize;
        let len = if nm != 0 { strlen(nm) } else { 0 };
        if len >= 9 && mem_eq(nm + len - 9, b"libc.so.6".as_ptr() as usize, 9) {
            LIBC_BASE = rd8(l + L_ADDR);
            return patch_bodies_of(LIBC_BASE, rd8(l + L_LD) as usize, base, interp);
        }
        l = rd8(l + L_NEXT) as usize;
        guard += 1;
    }
    0
}

unsafe fn patch_bodies_of(l_addr: u64, l_ld: usize, base: usize, interp: usize) -> usize {
    let mut done = 0;
    let limit = body_limit();
    let mut seen = 0usize;
    let count = Tables::count(interp);
    let mut i = 0;
    while i < count {
        let (name, len) = Tables::name(interp, i);
        let nm = base + name;
        if in_body_list(nm, len) && seen < limit {
            seen += 1;
            let at = find_sym(l_addr, l_ld, nm, len);
            if at != 0 {
                let ours = base + Tables::slot(interp, i);
                if at as usize != ours && patch_body(at as usize, ours) {
                    if debug2_on() {
                        say(&[
                            b"  body ",
                            core::slice::from_raw_parts(nm as *const u8, len),
                            b" at=",
                        ]);
                        say_hex(at as usize);
                        say(&[b" -> "]);
                        say_hex(ours);
                        say(&[b"\n"]);
                    }
                    done += 1;
                }
            }
        }
        i += 1;
    }
    if debug_on() {
        say(&[b"nr-boot: libc bodies="]);
        say_hex(done);
        say(&[b"\n"]);
    }
    done
}

unsafe fn patch_object(
    l_addr: u64,
    l_ld: usize,
    base: usize,
    interp: usize,
    count: usize,
) -> (usize, usize) {
    let mut done = 0;
    let mut failed = 0;
    if l_ld == 0 {
        return (0, 0);
    }
    let map_ptr = core::ptr::addr_of_mut!(MAP_SCRATCH);
    if !object_map(l_addr, l_ld, &mut *map_ptr) {
        return (0, 1);
    }
    let map = &*map_ptr;
    if !map.contains(l_ld, 16) {
        return (0, 1);
    }
    let strtab = norm_dyn(dyn_val(l_ld, DT_STRTAB, &map), l_addr, &map);
    let symtab = norm_dyn(dyn_val(l_ld, DT_SYMTAB, &map), l_addr, &map);
    let strsz = dyn_val(l_ld, DT_STRSZ, &map) as usize;
    // A dynamic object without these tables has nothing for us to patch. A
    // table with one missing is an ELF shape we do not understand and must not
    // silently accept.
    if strtab == 0 || symtab == 0 || strsz == 0 || !map.contains(strtab, strsz) {
        return (0, 1);
    }

    let tables = [(DT_RELA, DT_RELASZ), (DT_JMPREL, DT_PLTRELSZ)];
    let mut t = 0;
    while t < 2 {
        let addr = norm_dyn(dyn_val(l_ld, tables[t].0, &map), l_addr, &map);
        let size = dyn_val(l_ld, tables[t].1, &map) as usize;
        // glibc pre-adjusts DT_JMPREL/DT_STRTAB/DT_SYMTAB with l_addr but resolves
        // DT_RELA through D_PTR() at the use site, so DT_RELA stays relative.
        // norm_dyn() accepts either the absolute or l_addr-relative form only
        // when the resulting pointer lands inside this object's PT_LOAD map.
        t += 1;
        if debug2_on() {
            say(&[b"  table "]);
            say_hex((t - 1) as usize);
            say(&[b" addr="]);
            say_hex(addr);
            say(&[b" size="]);
            say_hex(size);
            say(&[b" l_addr="]);
            say_hex(l_addr as usize);
            say(&[b"\n"]);
        }
        if addr == 0 || size == 0 {
            continue;
        }
        if size > (1 << 28) || size % 24 != 0 {
            failed += 1;
            continue;
        }
        if !map.contains(addr, size) {
            failed += 1;
            continue;
        }
        let end = match addr.checked_add(size) {
            Some(v) => v,
            None => {
                failed += 1;
                continue;
            }
        };
        let mut p = addr;
        while p < end {
            if !map.contains(p, 24) {
                failed += 1;
                break;
            }
            let r_offset = rd8(p);
            let r_info = rd8(p + 8);
            let sym = (r_info >> 32) as usize;
            let kind = (r_info & 0xffff_ffff) as u32;
            p += 24;
            if sym == 0 {
                continue;
            }
            if kind != R_AARCH64_JUMP_SLOT && kind != R_AARCH64_GLOB_DAT {
                continue;
            }
            let sym_off = match sym.checked_mul(24) {
                Some(v) => v,
                None => {
                    failed += 1;
                    continue;
                }
            };
            let sp = match symtab.checked_add(sym_off) {
                Some(v) => v,
                None => {
                    failed += 1;
                    continue;
                }
            };
            if !map.contains(sp, 24) {
                failed += 1;
                continue;
            }
            let st_name = rd4(sp) as usize;
            if st_name == 0 {
                continue;
            }
            if st_name >= strsz {
                failed += 1;
                continue;
            }
            let cand_name = strtab + st_name;
            let mut i = 0;
            while i < count {
                let (name, len) = Tables::name(interp, i);
                let nm = base + name;
                if len < strsz - st_name
                    && map.contains(cand_name, len + 1)
                    && mem_eq(nm, cand_name, len)
                    && rd1(cand_name + len) == 0
                {
                    if debug2_on() {
                        say(&[
                            b"  hit ",
                            core::slice::from_raw_parts(cand_name as *const u8, len),
                            b"\n",
                        ]);
                    }
                    let slot = match l_addr.checked_add(r_offset) {
                        Some(v) => v as usize,
                        None => {
                            failed += 1;
                            break;
                        }
                    };
                    if !map.contains(slot, 8) {
                        failed += 1;
                        break;
                    }
                    let target = (base + Tables::slot(interp, i)) as u64;
                    if rd8(slot) != target {
                        if make_writable(slot, 8) {
                            wr8(slot, target);
                            done += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    break;
                }
                i += 1;
            }
        }
    }
    (done, failed)
}

/// Resolve a name at runtime for the rest of the shim (replaces dlsym on the
/// objects that matter). Far slower than dlsym, so callers must cache.
#[no_mangle]
pub unsafe extern "C" fn nr_resolve_name(main_map: usize, name: usize, len: usize) -> usize {
    resolve(main_map, name, len) as usize
}

#[no_mangle]
pub unsafe extern "C" fn nr_set_main_map(m: usize) {
    MAIN_MAP = m;
}
#[no_mangle]
pub unsafe extern "C" fn nr_get_main_map() -> usize {
    MAIN_MAP
}

// --------------------------------------------------------------- repatch
// Step 2 of `nr_boot` runs once, over the objects that exist at `_dl_init`.
// Anything loaded later resolves its own PLT against libc, so its `open()` went
// straight to the host filesystem: gdk-pixbuf's image loaders, GIO modules,
// libspa, GStreamer plugins, PAM/NSS modules. glibc appends newly loaded
// objects to the same link_map chain, so re-walking from MAIN_MAP after a
// successful dlopen covers them with no preload, no extra interposition and no
// syscall interception.
//
// proroot does the same thing in binary form: it interposes dlopen, dlsym,
// dl_iterate_phdr and re-scans the objects it has not seen yet (`proroot_patch_
// should_scan_object` in its export table).
static mut INTERP: usize = 0;
static mut INTERP_N: usize = 0;
static mut REPATCH_SEEN: [u64; 1024] = [0; 1024];
static mut REPATCH_SEEN_N: usize = 0;
static mut REPATCH_LOCK: u8 = 0;
static mut REPATCH_CALLS: u32 = 0;

unsafe fn repatch_seen(l_addr: u64) -> bool {
    let mut i = 0;
    while i < REPATCH_SEEN_N {
        if REPATCH_SEEN[i] == l_addr {
            return true;
        }
        i += 1;
    }
    false
}

unsafe fn repatch_remember(l_addr: u64) {
    if REPATCH_SEEN_N < 1024 {
        REPATCH_SEEN[REPATCH_SEEN_N] = l_addr;
        REPATCH_SEEN_N += 1;
    }
}

/// Re-apply the GOT interposition over every object reachable from the link
/// map. Idempotent: objects already patched are remembered by load bias.
#[no_mangle]
pub unsafe extern "C" fn nr_repatch() -> usize {
    if BOOTED == 0 || INTERP == 0 {
        return 0;
    }
    // Bisect knob: NR_NO_REPATCH=1 keeps dlopen'ed objects unpatched, which is
    // exactly the pre-evidence-100 behaviour. Needed because the first run with
    // repatch on ended in "double free or corruption" after the shell got past
    // JS init -- see evidence 102.
    if debug_var(b"NR_NO_REPATCH=1") {
        return 0;
    }
    if REPATCH_LOCK != 0 {
        return 0;
    }
    REPATCH_LOCK = 1;
    REPATCH_CALLS = REPATCH_CALLS.wrapping_add(1);
    let mut l = MAIN_MAP;
    let mut guard = 0;
    let mut objects = 0;
    let mut fresh = 0;
    let mut slots = 0;
    let mut failures = 0;
    while l != 0 && guard < 4096 {
        let l_addr = rd8(l + L_ADDR);
        let l_ld = rd8(l + L_LD) as usize;
        if !repatch_seen(l_addr) {
            repatch_remember(l_addr);
            let (n_here, failed_here) = patch_object(l_addr, l_ld, LOADER_BASE, INTERP, INTERP_N);
            slots += n_here;
            failures += failed_here;
            fresh += 1;
            if debug2_on() {
                let nm = rd8(l + L_NAME) as usize;
                say(&[b"nr-repatch: @"]);
                say_hex(l_addr as usize);
                say(&[b" "]);
                if nm != 0 {
                    say(&[core::slice::from_raw_parts(nm as *const u8, strlen(nm))]);
                } else {
                    say(&[b"?"]);
                }
                say(&[b"="]);
                say_hex(n_here);
                say(&[b" failed="]);
                say_hex(failed_here);
                say(&[b"\n"]);
            }
        }
        objects += 1;
        l = rd8(l + L_NEXT) as usize;
        guard += 1;
    }
    let rc = if l != 0 {
        // Longer than any walk we can trust: say so instead of stopping in the
        // middle and pretending the pass was complete.
        say(&[b"nr-repatch: link_map chain truncated\n"]);
        70
    } else {
        0
    };
    REPATCH_LOCK = 0;
    if debug_on() {
        say(&[b"nr-repatch: call="]);
        say_hex(REPATCH_CALLS as usize);
        say(&[b" objects="]);
        say_hex(objects);
        say(&[b" new="]);
        say_hex(fresh);
        say(&[b" slots="]);
        say_hex(slots);
        say(&[b" failed="]);
        say_hex(failures);
        say(&[b"\n"]);
    }
    rc
}
