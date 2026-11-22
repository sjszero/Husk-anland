package com.linuxhub.husk;

import android.app.Activity;
import android.graphics.Color;
import android.graphics.Typeface;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.text.method.ScrollingMovementMethod;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * Husk 0.1 - a shell for a guest Linux userland running inside this app process.
 *
 * It does exactly three things: find the guest rootfs, start the modified loader
 * under a PTY, and show the result. Everything else (the runtime) lives in the
 * loader itself.
 */
public class MainActivity extends Activity {

    private TextView out;
    private ScrollView scroll;
    private EditText input;
    private TextView status;

    private ParcelFileDescriptor pty;
    private FileOutputStream toGuest;
    private Thread reader;
    private volatile boolean alive;

    private static final int MAX_CHARS = 200_000;

    @Override protected void onCreate(Bundle b) {
        super.onCreate(b);
        setContentView(buildUi());
        new Thread(this::boot, "boot").start();
    }

    // ---------------------------------------------------------------- layout

    private View buildUi() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.parseColor("#0d1117"));

        status = new TextView(this);
        status.setTextSize(11);
        status.setTextColor(Color.parseColor("#8b98a5"));
        status.setPadding(dp(12), dp(8), dp(12), dp(6));
        status.setBackgroundColor(Color.parseColor("#161b22"));

        out = new TextView(this);
        out.setTypeface(Typeface.MONOSPACE);
        out.setTextSize(12);
        out.setTextColor(Color.parseColor("#d5dde5"));
        out.setPadding(dp(12), dp(8), dp(12), dp(8));
        out.setTextIsSelectable(true);
        out.setMovementMethod(new ScrollingMovementMethod());

        scroll = new ScrollView(this);
        scroll.addView(out, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT));
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f);
        scroll.setLayoutParams(lp);

        LinearLayout bar = new LinearLayout(this);
        bar.setOrientation(LinearLayout.HORIZONTAL);
        bar.setBackgroundColor(Color.parseColor("#161b22"));
        bar.setPadding(dp(8), dp(6), dp(8), dp(6));

        input = new EditText(this);
        input.setSingleLine(true);
        input.setHint("command");
        input.setTextColor(Color.parseColor("#e6edf3"));
        input.setHintTextColor(Color.parseColor("#5b6b7c"));
        input.setTypeface(Typeface.MONOSPACE);
        input.setTextSize(13);
        input.setImeOptions(EditorInfo.IME_ACTION_SEND);
        input.setBackgroundColor(Color.parseColor("#0d1117"));
        input.setPadding(dp(10), dp(6), dp(10), dp(6));
        input.setOnEditorActionListener((v, action, ev) -> {
            if (action == EditorInfo.IME_ACTION_SEND
                    || (ev != null && ev.getKeyCode() == KeyEvent.KEYCODE_ENTER
                        && ev.getAction() == KeyEvent.ACTION_DOWN)) {
                send(input.getText().toString() + "\n");
                input.setText("");
                return true;
            }
            return false;
        });
        bar.addView(input, new LinearLayout.LayoutParams(0,
                LinearLayout.LayoutParams.WRAP_CONTENT, 1f));

        Button ctrlC = new Button(this);
        ctrlC.setText("^C");
        ctrlC.setTextSize(12);
        ctrlC.setOnClickListener(v -> send("\u0003"));
        bar.addView(ctrlC, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT));

        root.addView(status);
        root.addView(scroll);
        root.addView(bar);
        root.setFitsSystemWindows(true);
        return root;
    }

    private int dp(int v) { return Math.round(v * getResources().getDisplayMetrics().density); }

    // ------------------------------------------------------------------ runtime

    private void boot() {
        String loader = new File(getApplicationInfo().nativeLibraryDir, "libld.so").getAbsolutePath();
        File rootfs = new File(getFilesDir(), "rootfs");
        String bash = new File(rootfs, "usr/bin/bash").getAbsolutePath();
        String libs = new File(rootfs, "usr/lib/aarch64-linux-gnu").getAbsolutePath();
        String home = new File(rootfs, "root").getAbsolutePath();
        String log = new File(getFilesDir(), "husk-child.log").getAbsolutePath();

        ui(status, "检查中");
        if (!new File(loader).exists() || !new File(bash).exists()) {
            ui(status, "未就绪");
            ui(out, "Husk 0.1\n\n缺少组件：\n  loader : " + loader
                    + (new File(loader).exists() ? "  ok" : "  缺失")
                    + "\n  rootfs : " + rootfs + (new File(bash).exists() ? "  ok" : "  缺失") + "\n");
            return;
        }
        new File(home).mkdirs();

        // 0) control: same loader, spawned the way the probe harness does it
        ui(out, probeViaProcessBuilder(loader, libs, bash, rootfs.getAbsolutePath()));

        // 1) non-interactive probe over a PTY
        ui(out, selfTest(loader, libs, bash, rootfs.getAbsolutePath(), home, log));

        // 2) the session itself
        startSession(loader, libs, bash, rootfs.getAbsolutePath(), home, log);
    }

    /** Control experiment: spawn the same loader the way the probe harness does. */
    private String probeViaProcessBuilder(String loader, String libs, String bash, String rootfs) {
        StringBuilder sb = new StringBuilder("== 对照：ProcessBuilder（无 PTY）==\n");
        try {
            ProcessBuilder pb = new ProcessBuilder(loader, "--library-path", libs, bash,
                    "-c", "echo HI; id; ls / | head -5");
            pb.redirectErrorStream(true);
            pb.directory(getFilesDir());
            pb.environment().put("NR_ROOTFS", rootfs);
            pb.environment().put("NR_LD", loader);
            pb.environment().put("PATH", "/usr/bin:/bin");
            Process pr = pb.start();
            byte[] buf = new byte[4096];
            long deadline = System.currentTimeMillis() + 5000;
            java.io.InputStream in = pr.getInputStream();
            while (System.currentTimeMillis() < deadline) {
                int n = in.read(buf);
                if (n <= 0) break;
                sb.append(new String(buf, 0, n, "UTF-8"));
            }
            sb.append("[status: ").append(describe(pr.waitFor())).append("]\n");
        } catch (Exception e) {
            sb.append("[exception: ").append(e).append("]\n");
        }
        return sb.toString();
    }

    private String selfTest(String loader, String libs, String bash, String rootfs,
                            String home, String log) {
        StringBuilder sb = new StringBuilder("== 自检：bash -c 'echo HI; id; pwd; ls /' ==\n");
        int fd = Pty.spawn(loader, libs, bash, rootfs, loader, 40, 100, home, log,
                sigsysLog(), new String[]{"-c", "echo HI; id; pwd; ls / | head -8"});
        if (fd < 0) return sb.append("spawn 失败\n").toString();

        ParcelFileDescriptor pfd = ParcelFileDescriptor.adoptFd(fd);
        try (FileInputStream in = new FileInputStream(pfd.getFileDescriptor())) {
            byte[] buf = new byte[4096];
            long deadline = System.currentTimeMillis() + 4000;
            while (System.currentTimeMillis() < deadline) {
                int n = in.read(buf);
                if (n <= 0) break;
                sb.append(clean(new String(buf, 0, n, "UTF-8")));
            }
        } catch (IOException e) {
            sb.append("[read: ").append(e.getMessage()).append("]\n");
        }
        int st = Pty.waitExit(-1);
        sb.append("[status: ").append(describe(st)).append("]\n");
        try { pfd.close(); } catch (IOException ignored) {}
        return sb.toString();
    }

    private void startSession(String loader, String libs, String bash, String rootfs,
                              String home, String log) {
        int fd = Pty.spawn(loader, libs, bash, rootfs, loader, 30, 80, home, log,
                sigsysLog(), new String[]{"-i"});
        if (fd < 0) { ui(status, "启动失败"); ui(out, "forkpty/exec 失败\n"); return; }

        pty = ParcelFileDescriptor.adoptFd(fd);
        toGuest = new FileOutputStream(pty.getFileDescriptor());
        alive = true;
        ui(status, "运行中 · loader " + new File(loader).getName());
        ui(out, "\n== 交互式会话 ==\n");

        reader = new Thread(this::readLoop, "pty-reader");
        reader.setDaemon(true);
        reader.start();

        // provocation: proves the write path and that bash answers on the pty
        new Thread(() -> {
            try { Thread.sleep(800); } catch (InterruptedException ignored) {}
            send("echo READY; uname -a; ls\n");
        }, "kick").start();
    }

    private String sigsysLog() {
        return new File(getFilesDir(), "sigsys.log").getAbsolutePath();
    }

    private static String describe(int st) {
        if (st == -2) return "仍在运行";
        if (st < 0) return "未知";
        if ((st & 0x7f) != 0) return "被信号 " + (st & 0x7f) + " 终止";
        return "退出码 " + ((st >> 8) & 0xff);
    }

    private void ui(TextView v, String s) { runOnUiThread(() -> { if (v == out) append(s); else v.setText(s); }); }

    private void readLoop() {
        byte[] buf = new byte[4096];
        try (FileInputStream in = new FileInputStream(pty.getFileDescriptor())) {
            int n;
            while (alive && (n = in.read(buf)) > 0) {
                final String s = clean(new String(buf, 0, n, "UTF-8"));
                runOnUiThread(() -> append(s));
            }
        } catch (IOException ignored) {
        }
        final int st = Pty.waitExit(-1);
        runOnUiThread(() -> {
            status.setText("会话已结束 · " + describe(st));
            append("\n[session ended: " + describe(st) + "]\n");
        });
    }

    private void send(String s) {
        if (toGuest == null) return;
        try { toGuest.write(s.getBytes("UTF-8")); toGuest.flush(); } catch (IOException ignored) {}
    }

    private void append(String s) {
        if (s == null || s.isEmpty()) return;
        CharSequence cur = out.getText();
        String merged = cur + s;
        if (merged.length() > MAX_CHARS) merged = merged.substring(merged.length() - MAX_CHARS);
        out.setText(merged);
        scroll.post(() -> scroll.fullScroll(View.FOCUS_DOWN));
    }

    /** Drop ANSI control sequences; keep the text a person can read. */
    private static String clean(String s) {
        s = s.replace("\r\n", "\n");
        s = s.replaceAll("\u001b\\[[0-9;?]*[ -/]*[@-~]", "");   // CSI
        s = s.replaceAll("\u001b\\][^\u0007]*\u0007", "");       // OSC
        s = s.replaceAll("\u001b[()][A-Za-z0-9]", "");
        s = s.replace("\u0007", "");
        return s;
    }

    @Override protected void onDestroy() {
        alive = false;
        send("exit\n");
        try { if (pty != null) pty.close(); } catch (IOException ignored) {}
        super.onDestroy();
    }
}
