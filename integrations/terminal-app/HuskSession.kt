package com.terminal

import android.content.Context
import java.io.File

/**
 * Husk: run a glibc Linux userland as this app's own process.
 *
 * The runtime is the modified guest loader shipped as jniLibs/libld.so - path
 * mapping, the build-time removal of the two seccomp-trapped startup calls and
 * the symbol interposition all live inside it. This object only decides whether a
 * guest rootfs is present and what argv/env the session needs.
 */
object HuskSession {

    class Launch(
        val command: String,
        val arguments: Array<String>,
        val environment: Array<String>,
        val workingDirectory: String?
    )

    fun resolve(context: Context): Launch? {
        val rootfs = File(context.filesDir, "rootfs")
        val bash = File(rootfs, "usr/bin/bash")
        val libs = File(rootfs, "usr/lib/aarch64-linux-gnu")
        val loader = File(context.applicationInfo.nativeLibraryDir, "libld.so")
        if (!loader.exists() || !bash.exists() || !libs.isDirectory) return null

        return Launch(
            command = loader.absolutePath,
            arguments = arrayOf("--library-path", libs.absolutePath, bash.absolutePath, "-i"),
            environment = arrayOf(
                "NR_ROOTFS=${rootfs.absolutePath}",
                "NR_LD=${loader.absolutePath}",
                "TERM=xterm-256color",
                "LANG=C.UTF-8",
                "USER=root",
                "HOME=/root",
                "SHELL=/usr/bin/bash",
                "PATH=/usr/bin:/bin:/usr/sbin:/sbin"
            ),
            workingDirectory = null
        )
    }
}
