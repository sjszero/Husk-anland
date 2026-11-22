/*
 * LinuxHub Native Runtime - loader entry hook for the guest ld.so.
 *
 * Why this has to live in ld.so's own entry point:
 * glibc's __pthread_initialize_minimal_internal (inside ld.so, before any
 * relocation-driven constructor runs) calls
 *     set_tid_address(96) -> set_robust_list(99) -> [rseq(293)]
 * Android's untrusted_app seccomp policy TRAPs 99 and 293, so the process dies
 * with SIGSYS (exit 159) before LD_PRELOAD code can install anything. There is
 * no earlier hook in user space: this stub is it.
 *
 * It installs a SIGSYS handler that turns a trapped syscall into -ENOSYS and
 * steps over the `svc`, which is exactly the answer libc documents as "kernel
 * does not have it, take the fallback path". Cost when nothing traps: one
 * rt_sigaction at startup, zero per call. No ptrace, no seccomp user notify.
 *
 * Layout: this blob is placed at NR_STUB_VADDR inside the loader's first
 * (RX) PT_LOAD and ld.so's _start has its leading `nop` replaced by a branch
 * to it; the stub returns to _start+4. e_entry is deliberately NOT touched:
 * glibc decides "the loader is the program" by comparing auxv AT_ENTRY with
 * &_start, and repointing e_entry makes it take the interpreter path and
 * assert in rtld_setup_main_map.
 */
#define NR_STUB_VADDR 0x26730
#define NR_ORIG_ENTRY 0x1aa44

	.text
	.balign 8
	.globl	nr_entry_stub
nr_entry_stub:
.Lbase:
	/* preserve everything the real _start might care about */
	stp	x0, x1, [sp, #-96]!
	stp	x2, x3, [sp, #16]
	stp	x4, x5, [sp, #32]
	stp	x6, x7, [sp, #48]
	stp	x8, x9, [sp, #64]
	/* kern_sigaction{HANDLER, SA_SIGINFO|SA_RESTART, 0, mask 0} on the stack */
	sub	sp, sp, #32
	adr	x2, nr_sigsys_handler
	movz	x3, #0x0004
	movk	x3, #0x1000, lsl #16
	stp	x2, x3, [sp]
	stp	xzr, xzr, [sp, #16]
	mov	x0, #31			/* SIGSYS */
	mov	x1, sp
	mov	x2, #0			/* oldact */
	mov	x3, #8			/* sizeof(sigset_t) */
	mov	x8, #134		/* rt_sigaction */
	svc	#0
	add	sp, sp, #32
	ldp	x8, x9, [sp, #64]
	ldp	x6, x7, [sp, #48]
	ldp	x4, x5, [sp, #32]
	ldp	x2, x3, [sp, #16]
	ldp	x0, x1, [sp], #96
	/* br original entry, computed from our own runtime pc */
	adr	x17, .Lbase
	ldr	x16, nr_lit_stub
	sub	x17, x17, x16		/* load bias */
	ldr	x16, nr_lit_orig
	add	x16, x17, x16
	br	x16

/*
 * SIGSYS handler. Records 16 bytes on fd 2 per trap:
 *   u32 si_syscall, u32 0, u64 offset-of-uc_mcontext.pc
 * then makes the syscall return -ENOSYS by writing regs[0] in the sigframe and
 * stepping pc past the `svc`.
 *
 * The ucontext layout is *found*, not assumed: the kernel puts the call
 * instruction address in si_call_addr, and the same value is sitting in
 * uc_mcontext.pc, so we scan for it and require the following word to be a
 * plausible pstate (high half zero). That makes this immune to the
 * uapi/glibc ucontext ordering differences.
 */
nr_sigsys_handler:
	stp	x1, x2, [sp, #-96]!
	stp	x3, x4, [sp, #16]
	stp	x5, x6, [sp, #32]
	stp	x7, x8, [sp, #48]
	ldr	w3, [x1, #24]		/* si_syscall */
	ldr	x4, [x1, #16]		/* si_call_addr */
	str	w3, [sp, #64]
	str	wzr, [sp, #68]
	mov	x5, #32
nr_scan:
	ldr	x6, [x2, x5]
	cmp	x6, x4
	b.ne	nr_scan_next
	add	x7, x5, #8
	ldr	x8, [x2, x7]
	lsr	x8, x8, #32
	cbnz	x8, nr_scan_next
	b	nr_scan_done
nr_scan_next:
	add	x5, x5, #4
	movz	x8, #8192
	cmp	x5, x8
	b.lo	nr_scan
	mov	x5, xzr		/* not found */
nr_scan_done:
	str	x5, [sp, #72]
	mov	x0, #2			/* fd 2: the log record */
	add	x1, sp, #64
	mov	x2, #16
	mov	x8, #64			/* write */
	svc	#0
	cbz	x5, nr_fatal		/* could not find the frame: do not loop */
	sub	x7, x5, #256		/* uc_mcontext.pc - 264 + 8 = regs[0] */
	mov	x6, #-38		/* -ENOSYS */
	str	x6, [x2, x7]
	add	x6, x4, #4
	str	x6, [x2, x5]
	b	nr_handler_ret
nr_fatal:
	mov	x0, #159
	mov	x8, #94			/* exit_group */
	svc	#0
nr_handler_ret:
	ldp	x7, x8, [sp, #48]
	ldp	x5, x6, [sp, #32]
	ldp	x3, x4, [sp, #16]
	ldp	x1, x2, [sp], #96
	ret

	.balign 8
nr_lit_stub:
	.quad	NR_STUB_VADDR
nr_lit_orig:
	.quad	NR_ORIG_ENTRY
