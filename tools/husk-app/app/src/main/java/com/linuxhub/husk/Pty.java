package com.linuxhub.husk;

/** Thin JNI surface over forkpty(3). See app/src/main/jni/pty.c. */
final class Pty {
    static { System.loadLibrary("huskpty"); }

    private Pty() {}

    /** @return master fd, or -1 on failure */
    static native int spawn(String loader, String libraryPath, String exe,
                            String rootfs, String ld, int rows, int cols,
                            String home, String logPath);

    static native int resize(int fd, int rows, int cols);

    static native int waitExit(int fd);
}
