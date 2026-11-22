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
        start();
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

    private void start() {
        String loader = new File(getApplicationInfo().nativeLibraryDir, "libld.so").getAbsolutePath();
        File rootfs = new File(getFilesDir(), "rootfs");
        String bash = new File(rootfs, "usr/bin/bash").getAbsolutePath();
        String libs = new File(rootfs, "usr/lib/aarch64-linux-gnu").getAbsolutePath();
        String home = new File(rootfs, "root").getAbsolutePath();

        if (!new File(loader).exists() || !new File(bash).exists()) {
            status.setText("未就绪");
            append("Husk 0.1\n\n"
                    + "缺少组件：\n"
                    + "  loader : " + loader + (new File(loader).exists() ? "  ok" : "  缺失") + "\n"
                    + "  rootfs : " + rootfs + (new File(bash).exists() ? "  ok" : "  缺失") + "\n\n"
                    + "rootfs 需要一个 glibc 发行版目录树，例如 Ubuntu 24.04 arm64，\n"
                    + "放在 " + rootfs + "\n");
            return;
        }
        new File(home).mkdirs();

        int rows = 30, cols = 80;
        String log = new File(getFilesDir(), "husk-child.log").getAbsolutePath();
        int fd = Pty.spawn(loader, libs, bash, rootfs.getAbsolutePath(), loader, rows, cols, home, log);
        if (fd < 0) {
            status.setText("启动失败");
            append("forkpty/exec 失败\n");
            return;
        }
        pty = ParcelFileDescriptor.adoptFd(fd);
        toGuest = new FileOutputStream(pty.getFileDescriptor());
        alive = true;

        status.setText("运行中 · " + rootfs.getName() + " · loader " + new File(loader).getName());
        append("Husk 0.1 — 输入命令后按回车\n\n");

        reader = new Thread(this::readLoop, "pty-reader");
        reader.setDaemon(true);
        reader.start();
    }

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
            String why;
            if (st < 0) why = "状态未知";
            else if ((st & 0x7f) != 0) why = "被信号 " + (st & 0x7f) + " 终止";
            else why = "退出码 " + ((st >> 8) & 0xff);
            status.setText("会话已结束 · " + why);
            append("\n[session ended: " + why + "]\n");
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
