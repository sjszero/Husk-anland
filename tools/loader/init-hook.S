/*
 * _dl_init hook for the loader-embedded LinuxHub path shim.
 *
 * Reached by repointing the `bl _dl_init` inside ld.so's _start (0x1aa74 ->
 * 0x4bb0 in glibc 2.39). At that point every object has been relocated and
 * RELRO-protected, and no constructor has run yet, so this is the earliest point
 * at which the GOT of the loaded objects can be rewritten for interposition.
 *
 * Nothing here is hard-coded: nr-tools embed fills nr_hook_data and the descriptor
 * page with loader-relative addresses, and the code finds its own load bias with
 * adr/sub, so it is position independent and relocation free.
 *
 * Descriptor (loader-relative, 64-bit fields):
 *    0  extern table      (names of the symbols this object imports)
 *    8  interpose table   (names this object takes over)
 *   16  nr_boot address   (signature: base, main_map, externs, interpose, envp)
 *   24  rw_start  - first byte of this object's writable segment
 *   32  rw_len
 *   40  self-relocation slots (every R_AARCH64_RELATIVE target)
 */
	.text
	.balign 8
	.globl	nr_dl_init_hook
nr_dl_init_hook:
.Lself:
	stp	x29, x30, [sp, #-64]!
	mov	x29, sp
	stp	x0, x1, [sp, #16]
	stp	x2, x3, [sp, #32]
	stp	x19, x20, [sp, #48]

#if NR_FAULT
	/* Diagnostic build: install a fault handler so a crash inside the embedded
	 * object reports si_addr plus the saved register frame. This ROM writes no
	 * tombstone for these child processes, so without it a fault is invisible.
	 * It is a reporter only: it never resumes execution. */
	sub	sp, sp, #32
	adr	x2, nr_fault
	movz	x3, #0x0004
	movk	x3, #0x1000, lsl #16		/* SA_SIGINFO|SA_RESTART */
	stp	x2, x3, [sp]
	stp	xzr, xzr, [sp, #16]
	mov	x0, #11				/* SIGSEGV */
	mov	x1, sp
	mov	x2, #0
	mov	x3, #8
	mov	x8, #134
	svc	#0
	mov	x0, #7				/* SIGBUS */
	mov	x1, sp
	mov	x2, #0
	mov	x3, #8
	mov	x8, #134
	svc	#0
	mov	x0, #4				/* SIGILL */
	mov	x1, sp
	mov	x2, #0
	mov	x3, #8
	mov	x8, #134
	svc	#0
	add	sp, sp, #32
#endif

	adr	x19, .Lself
	ldr	x20, nr_hook_data+8		/* our own vaddr */
	sub	x19, x19, x20			/* x19 = loader load bias */
	ldr	x20, nr_hook_data+16		/* descriptor vaddr */
	add	x20, x19, x20

	/* the embedded object arrives read-only; its .data/.bss/GOT must be
	 * writable before nr_boot touches anything, including its own statics.
	 *
	 * The shim is linked with a 4 KiB maximum page size, so its writable
	 * segment begins on a 4 KiB boundary - which is not necessarily a 16 KiB
	 * or 64 KiB one, and mprotect() returns EINVAL for a start address that is
	 * not aligned to the *kernel's* page size. The kernel page size is not
	 * known at this point (the auxv has not been read yet), so ask for 4, then
	 * 16, then 64 KiB: every one of those is a legal request on any of those
	 * kernels, and the kernel accepts exactly the ones it can honour. Ignoring
	 * the result here is what turned a 16 KiB kernel into a silent SIGSEGV on
	 * the first write to our own statics. */
	ldr	x5, [x20, #24]
	add	x5, x19, x5			/* rw_start, absolute */
	ldr	x6, [x20, #32]
	add	x6, x5, x6			/* rw_end, absolute */
	mov	x9, #4096
.Lmp_try:
	neg	x10, x9
	sub	x11, x9, #1
	and	x0, x5, x10			/* start rounded down */
	add	x1, x6, x11
	and	x1, x1, x10			/* end rounded up */
	sub	x1, x1, x0
	mov	x2, #3				/* PROT_READ|PROT_WRITE */
	mov	x8, #226			/* mprotect */
	svc	#0
	cbz	x0, .Lmp_ok
	lsl	x9, x9, #2			/* 4 KiB -> 16 KiB -> 64 KiB */
	cmp	x9, #0x10000
	b.ls	.Lmp_try
	/* no page size worked: fail loudly, never interpose a read-only object */
	adr	x0, .Lmsg_mp
	mov	x1, #(.Lmsg_mp_end - .Lmsg_mp)
	mov	x2, #2
	mov	x8, #64				/* write */
	svc	#0
	mov	x0, #98
	mov	x8, #94				/* exit_group */
	svc	#0
	b	.
.Lmp_ok:

	mov	x0, x19				/* base */
	ldr	x1, [sp, #16]			/* main_map */
	ldr	x2, [x20]			/* extern table */
	add	x2, x19, x2
	ldr	x3, [x20, #8]			/* interpose table */
	add	x3, x19, x3
	ldr	x4, [sp, #40]			/* envp (x3 saved at sp+40) */
	ldr	x5, [x20, #40]			/* self-relocation table */
	add	x5, x19, x5
	ldr	x16, [x20, #16]			/* nr_boot */
	add	x16, x19, x16
	blr	x16
	cbnz	x0, .Lboot_fail

	ldp	x19, x20, [sp, #48]
	ldp	x2, x3, [sp, #32]
	ldp	x0, x1, [sp, #16]
	ldp	x29, x30, [sp], #64

	adr	x9, .Lself
	ldr	x10, nr_hook_data+8
	sub	x9, x9, x10
	ldr	x16, nr_hook_data+24		/* real _dl_init */
	add	x16, x9, x16
	br	x16

.Lboot_fail:
	/* nr_boot returned a non-zero step code. Never enter the guest with a
	 * partially interposed GOT: exit_group(x0) makes the failure visible and
	 * prevents an unpatched path from escaping the guest root. */
	mov	x8, #94				/* exit_group */
	svc	#0
	b	.

/* Fault reporter: x1 = siginfo*, x2 = ucontext*.
 * Writes "FAULT" + u32 signo + u32 code + u64 si_addr + 512 bytes of the saved
 * register frame starting at ucontext+176, then exit_group(99). It never
 * returns and never tries to continue. */
nr_fault:
	sub	sp, sp, #544
	ldr	w3, [x1]			/* si_signo */
	str	w3, [sp]
	ldr	w3, [x1, #8]			/* si_code */
	str	w3, [sp, #4]
	ldr	x3, [x1, #16]			/* si_addr */
	str	x3, [sp, #8]
	add	x4, x2, #176			/* uc_mcontext */
	mov	x5, xzr
1:	cmp	x5, #512
	b.hs	2f
	ldr	x6, [x4, x5]
	add	x7, sp, #16
	str	x6, [x7, x5]
	add	x5, x5, #8
	b	1b
2:	mov	x0, #2
	mov	x1, sp
	mov	x2, #528
	mov	x8, #64
	svc	#0
	mov	x0, #99
	mov	x8, #94				/* exit_group */
	svc	#0
	b	.


.Lmsg_mp:
	.ascii "nr-hook: mprotect of the shim's writable segment failed at 4K, 16K and 64K\n"
.Lmsg_mp_end:
	.balign 8
nr_hook_data:
	.quad	0x4e52484b4f4f4b31		/* "NRHOOK1" */
	.quad	0				/* self vaddr     */
	.quad	0				/* descriptor     */
	.quad	0				/* _dl_init       */
