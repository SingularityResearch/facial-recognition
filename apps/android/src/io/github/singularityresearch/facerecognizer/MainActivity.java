package io.github.singularityresearch.facerecognizer;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.AssetManager;
import android.net.Uri;
import android.graphics.Color;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.PermissionRequest;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The Android shell: one window that is the system's web view, showing the tracker page that the Rust side
 * serves inside this app on 127.0.0.1. Everything the app does is in the pages and in Rust; this class only
 * starts them and answers the web view's question about the camera.
 */
public final class MainActivity extends Activity {
    private WebView web;
    /** The app's own server, once it runs. */
    private String origin;
    /** A join code the phone's camera app opened the app with, until the join page has it. */
    private String joinCode;

    /** The loading screen, while it is shown. */
    private View loading;

    /**
     * The logo, the app's name and a line, over the still empty web view while the models are unpacked and
     * loaded (seconds; the first start after an install longer). Taken away when the page has loaded.
     */
    private View loadingScreen() {
        float dp = getResources().getDisplayMetrics().density;
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setGravity(Gravity.CENTER);
        box.setBackgroundColor(Color.BLACK);
        ImageView logo = new ImageView(this);
        logo.setImageResource(getApplicationInfo().icon);
        box.addView(logo, new LinearLayout.LayoutParams((int) (120 * dp), (int) (120 * dp)));
        TextView title = new TextView(this);
        title.setText("Face Recognizer");
        title.setTextColor(Color.WHITE);
        title.setTextSize(24);
        title.setGravity(Gravity.CENTER);
        title.setPadding(0, (int) (22 * dp), 0, (int) (10 * dp));
        box.addView(title);
        TextView note = new TextView(this);
        note.setText("Loading the models\u2026");
        note.setTextColor(Color.rgb(178, 178, 178));
        note.setTextSize(15);
        note.setGravity(Gravity.CENTER);
        note.setPadding(0, 0, 0, (int) (14 * dp));
        box.addView(note);
        box.addView(new ProgressBar(this));
        return box;
    }

    private void hideLoading() {
        if (loading != null) {
            ((ViewGroup) loading.getParent()).removeView(loading);
            loading = null;
        }
    }

    /** The join code of an intent (facetracker-join:...), or null. */
    private static String joinCodeOf(Intent intent) {
        Uri data = intent == null ? null : intent.getData();
        return data != null && "facetracker-join".equals(data.getScheme()) ? data.toString() : null;
    }

    /** The join page, asked about the code: nothing is done with a code before the owner says so there. */
    private void openJoin(String code) {
        web.loadUrl(origin + "/join?code=" + Uri.encode(code));
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        String code = joinCodeOf(intent);
        if (code == null) {
            return;
        }
        if (origin != null) {
            openJoin(code);
        } else {
            joinCode = code;
        }
    }

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        joinCode = joinCodeOf(getIntent());
        // a camera that is watched should not have its screen go dark
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        web = new WebView(this);
        web.setBackgroundColor(Color.BLACK);                 // dark until its page is there: an empty one is white
        FrameLayout frame = new FrameLayout(this);
        frame.setBackgroundColor(Color.BLACK);
        frame.addView(web, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        loading = loadingScreen();
        frame.addView(loading, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(frame);
        // the camera for the tracker; the microphone for its voice signals and the Signals chart's sound, which
        // the page uses only while those options are on (asked for once, together)
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
                && checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            start();
        } else {
            requestPermissions(new String[] {Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO}, 1);
        }
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        start();        // without the camera or the microphone the page says so itself
    }

    /**
     * How warm the phone says it is, its battery, and whether it is on power, handed to the app every few
     * seconds for the Signals chart: numbers and a state, of which nothing is kept. Android has seven heat
     * states and the chart four: none; light and moderate; severe; critical and beyond. A phone gives no degrees.
     */
    private final android.os.Handler healthTimer = new android.os.Handler(android.os.Looper.getMainLooper());
    private final Runnable sayHealth = new Runnable() {
        @Override
        public void run() {
            int heat = -1;
            if (android.os.Build.VERSION.SDK_INT >= 29) {
                android.os.PowerManager power = (android.os.PowerManager) getSystemService(POWER_SERVICE);
                int status = power == null ? -1 : power.getCurrentThermalStatus();
                heat = status < 0 ? -1 : status == 0 ? 0 : status <= 2 ? 1 : status == 3 ? 2 : 3;
            }
            float level = -1;
            int onPower = -1;
            android.os.BatteryManager battery = (android.os.BatteryManager) getSystemService(BATTERY_SERVICE);
            if (battery != null) {
                int percent = battery.getIntProperty(android.os.BatteryManager.BATTERY_PROPERTY_CAPACITY);
                if (percent >= 0 && percent <= 100) {
                    level = percent;
                }
            }
            // (the system's own standing notice of the battery: read, never waited for)
            Intent now = registerReceiver(null, new android.content.IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            if (now != null) {
                int plugged = now.getIntExtra(android.os.BatteryManager.EXTRA_PLUGGED, -1);
                onPower = plugged < 0 ? -1 : plugged != 0 ? 1 : 0;
            }
            try {
                Native.health(heat, level, onPower);
            } catch (UnsatisfiedLinkError e) {
                return;                                    // (an app without its library says nothing)
            }
            healthTimer.postDelayed(this, 5000);
        }
    };

    private void start() {
        new Thread(() -> {
            // The models and the page files are in the app's package; Rust and the model runtime read
            // files, so they are unpacked into the app's own folder once (and again when one changed size).
            File root = new File(getFilesDir(), "root");
            String why = null;
            int port = -1;
            try {
                unpack("models", new File(root, "models"));
                unpack("pages", new File(root, "pages"));
                // what the shared database's other devices see this one as, once it joins one
                String name = android.provider.Settings.Global.getString(getContentResolver(), "device_name");
                if (name == null || name.trim().isEmpty()) {
                    name = android.os.Build.MODEL;
                }
                Native.device(name, "Android " + android.os.Build.VERSION.RELEASE);
                healthTimer.post(sayHealth);
                port = Native.start(root.getPath(), new File(getFilesDir(), "home").getPath(),
                        new File(root, "pages").getPath(), getApplicationInfo().nativeLibraryDir + "/libonnxruntime.so");
                if (port <= 0) {
                    why = Native.error();
                }
            } catch (IOException | UnsatisfiedLinkError e) {
                why = String.valueOf(e);
            }
            final int at = port;
            final String failed = why;
            runOnUiThread(() -> {
                if (at > 0) {
                    load(at);
                } else {
                    TextView text = new TextView(this);
                    text.setText("The tracker didn't start: " + failed);
                    setContentView(text);
                }
            });
        }, "start").start();
    }

    private void load(int port) {
        origin = "http://127.0.0.1:" + port;
        final String origin = this.origin;
        WebSettings settings = web.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setMediaPlaybackRequiresUserGesture(false);       // the camera's picture starts without a tap
        web.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageFinished(WebView view, String url) {
                hideLoading();
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            // The page asks for the camera or the microphone: the app's own page, served by the app itself,
            // gets what it asked for of the two, as far as the owner allowed the app each when it first
            // started. Any other page gets nothing, and nothing else is ever given.
            @Override
            public void onPermissionRequest(PermissionRequest request) {
                String from = String.valueOf(request.getOrigin());
                boolean own = from.equals(origin) || from.equals(origin + "/");
                java.util.ArrayList<String> given = new java.util.ArrayList<>();
                if (own) {
                    for (String wanted : request.getResources()) {
                        if (PermissionRequest.RESOURCE_VIDEO_CAPTURE.equals(wanted)
                                && checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                            given.add(wanted);
                        }
                        if (PermissionRequest.RESOURCE_AUDIO_CAPTURE.equals(wanted)
                                && checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                            given.add(wanted);
                        }
                    }
                }
                if (given.isEmpty()) {
                    request.deny();
                } else {
                    request.grant(given.toArray(new String[0]));
                }
            }
        });
        if (joinCode != null) {
            openJoin(joinCode);
            joinCode = null;
        } else {
            web.loadUrl(origin + "/");
        }
    }

    /** Copy a folder of the package's assets to a folder of files, each file whose size differs. */
    private void unpack(String from, File to) throws IOException {
        AssetManager assets = getAssets();
        String[] names = assets.list(from);
        if (names == null || names.length == 0) {
            return;
        }
        if (!to.isDirectory() && !to.mkdirs()) {
            throw new IOException("can't make " + to);
        }
        for (String name : names) {
            String inner = from + "/" + name;
            String[] below = assets.list(inner);
            if (below != null && below.length > 0) {
                unpack(inner, new File(to, name));
                continue;
            }
            File out = new File(to, name);
            try (InputStream in = assets.open(inner)) {
                File part = new File(to, name + ".part");
                long size = 0;
                try (OutputStream o = new FileOutputStream(part)) {
                    byte[] buffer = new byte[1 << 16];
                    for (int n; (n = in.read(buffer)) > 0; size += n) {
                        o.write(buffer, 0, n);
                    }
                }
                if (out.isFile() && out.length() == size) {
                    part.delete();
                } else if (!part.renameTo(out)) {
                    throw new IOException("can't write " + out);
                }
            }
        }
    }

    @Override
    protected void onDestroy() {
        healthTimer.removeCallbacks(sayHealth);
        web.destroy();
        super.onDestroy();
    }
}
