package com.linuxhub.probe;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import java.io.File;
import java.io.InputStream;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;

/**
 * Probe harness. Runs a native executable from nativeLibraryDir inside a real
 * app process (untrusted_app domain) and captures its output.
 *
 * extras:
 *   exe   - file name inside nativeLibraryDir (default: libprobe.so)
 *   args  - whitespace separated arguments for that executable
 *   root  - if set, exports NR_ROOTFS=files/<root> and NR_LD=<nativeLibraryDir>/libld.so
 *   env   - extra KEY=VALUE pairs joined by commas
 */
public class MainActivity extends Activity {
    private static final String TAG = "NRProbe";

    @Override protected void onCreate(Bundle b) {
        super.onCreate(b);
        Thread t = new Thread(() -> {
            File out = new File(getFilesDir(), "probe.txt");
            File err = new File(getFilesDir(), "spawn.txt");
            try {
                String libDir = getApplicationInfo().nativeLibraryDir;
                String exeName = getIntent().getStringExtra("exe");
                // "hostexe" runs an absolute path instead (used to measure a
                // bionic baseline in exactly the same process/uid/domain).
                String hostExe = getIntent().getStringExtra("hostexe");
                File exe;
                if (hostExe != null && !hostExe.isEmpty()) {
                    exe = new File(hostExe);
                } else {
                    if (exeName == null || exeName.isEmpty()) exeName = "libprobe.so";
                    exe = new File(libDir, exeName);
                }
                String extra = getIntent().getStringExtra("args");
                java.util.List<String> cmd = new java.util.ArrayList<>();
                cmd.add(exe.getAbsolutePath());
                if (extra != null && !extra.isEmpty()) {
                    for (String a : extra.trim().split("\\s+")) if (!a.isEmpty()) cmd.add(a);
                }
                ProcessBuilder pb = new ProcessBuilder(cmd);
                pb.redirectErrorStream(true);
                pb.directory(getFilesDir());
                String root = getIntent().getStringExtra("root");
                if (root != null && !root.isEmpty()) {
                    pb.environment().put("NR_ROOTFS", new File(getFilesDir(), root).getAbsolutePath());
                    // No LD_PRELOAD: the path shim lives inside the loader itself
                    // (one extra PT_LOAD, entered from a _dl_init hook).
                    pb.environment().put("NR_LD", new File(libDir, "libld.so").getAbsolutePath());
                    pb.environment().put("PATH", "/usr/bin:/bin");
                    // glibc 2.35+ registers rseq at startup; this device's
                    // seccomp policy kills rseq (SIGSYS), so ask glibc not to.
                    String tun = getIntent().getStringExtra("tunables");
                    pb.environment().put("GLIBC_TUNABLES", tun == null ? "glibc.pthread.rseq=0" : tun);
                    // seccomp SIGSYS catcher in the shim: log the trapped
                    // syscall number, or turn the trap into -ENOSYS.
                    String ss = getIntent().getStringExtra("sigsys");
                    pb.environment().put("NR_SIGSYS", ss == null ? "log" : ss);
                    pb.environment().put("NR_SIGSYS_LOG",
                            new File(getFilesDir(), "sigsys.txt").getAbsolutePath());
                }
                // extra env, "K=V,K2=V2"
                String envExtra = getIntent().getStringExtra("env");
                if (envExtra != null && !envExtra.isEmpty()) {
                    for (String kv : envExtra.split(",")) {
                        int eq = kv.indexOf('=');
                        if (eq > 0) pb.environment().put(kv.substring(0, eq), kv.substring(eq + 1));
                    }
                }
                Process p = pb.start();
                try (InputStream in = p.getInputStream(); FileOutputStream fo = new FileOutputStream(out)) {
                    in.transferTo(fo);
                }
                int code = p.waitFor();
                try (java.io.FileOutputStream fo2 = new java.io.FileOutputStream(out, true)) {
                    fo2.write(("\n#exit=" + code + "\n").getBytes(StandardCharsets.UTF_8));
                }
                Log.i(TAG, "probe finished: " + out.length() + " bytes, exit=" + code);
            } catch (Throwable e) {
                try (FileOutputStream fo = new FileOutputStream(err)) {
                    fo.write((e.toString() + "\n").getBytes(StandardCharsets.UTF_8));
                } catch (Exception ignored) {}
                Log.e(TAG, "probe failed", e);
            }
            runOnUiThread(MainActivity.this::finish);
        });
        t.start();
    }
}
