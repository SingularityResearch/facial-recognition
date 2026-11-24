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
        if (code == 1) {
            start();    // without the camera or the microphone the page says so itself
        }
        // (the location's answer, code 2: its step sees it the next time it looks)
    }

    /**
     * The phone's own location service, asked only while the owner has switched the location on in the page (the
     * Rust side says so). Android asks the owner then, never at the app's start. Each location is handed to the
     * Rust side as numbers, and why there is none as a number too. Nothing is kept here.
     */
    private boolean locating;
    private boolean locationAsked;
    /** What the system last said, and the steps since: said again every minute while the phone stays put. */
    private android.location.Location lastPlace;
    private int quietSteps;
    private final android.location.LocationListener whereabouts = new android.location.LocationListener() {
        @Override
        public void onLocationChanged(android.location.Location at) {
            if (at == null || !at.hasAccuracy()) {
                return;
            }
            lastPlace = at;
            quietSteps = 0;
            try {
                Native.location(at.getLatitude(), at.getLongitude(), at.getAccuracy());
            } catch (UnsatisfiedLinkError e) {
                // (an app without its library takes nothing)
            }
        }

        @Override
        public void onProviderEnabled(String provider) {
        }

        @Override
        public void onProviderDisabled(String provider) {
            lastPlace = null;                               // (no longer said again: the system stopped telling)
        }

        @Override
        public void onStatusChanged(String provider, int status, Bundle extras) {
        }
    };
    private final Runnable locationStep = new Runnable() {
        @Override
        public void run() {
            boolean wanted;
            try {
                wanted = Native.locationWanted();
            } catch (UnsatisfiedLinkError e) {
                return;
            }
            android.location.LocationManager places = (android.location.LocationManager) getSystemService(LOCATION_SERVICE);
            if (wanted && !locating && places != null) {
                boolean fine = checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED;
                boolean coarse = checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED;
                if (!fine && !coarse) {
                    if (!locationAsked) {
                        locationAsked = true;                // asked once for each time the switch is turned on
                        Native.locationRefused(1);
                        requestPermissions(new String[] {Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION}, 2);
                    } else {
                        Native.locationRefused(2);
                    }
                } else if (android.os.Build.VERSION.SDK_INT >= 28 && !places.isLocationEnabled()) {
                    Native.locationRefused(3);
                } else {
                    locating = listen(places, fine);
                    Native.locationRefused(locating ? 1 : 4);
                }
            } else if (!wanted) {
                if (locating && places != null) {
                    places.removeUpdates(whereabouts);
                }
                locating = false;
                locationAsked = false;
                lastPlace = null;
            } else if (locating && lastPlace != null && ++quietSteps >= 20) {
                // A phone that stays where it is gets no new location from the system (it reports a move of
                // 20 m and more): what it last said is said again every minute, so that the place's time says
                // the phone is still there.
                quietSteps = 0;
                Native.location(lastPlace.getLatitude(), lastPlace.getLongitude(), lastPlace.getAccuracy());
            }
            healthTimer.postDelayed(this, 3000);
        }
    };

    /**
     * Ask the system to say where the phone is, and again when it has moved: its own combined provider where the
     * system has one (Android 12 and later), else the satellites (only with the fine permission) and the networks.
     * Whether any provider was asked.
     */
    private boolean listen(android.location.LocationManager places, boolean fine) {
        java.util.List<String> there = places.getAllProviders();
        String[] ask = android.os.Build.VERSION.SDK_INT >= 31 && there.contains(android.location.LocationManager.FUSED_PROVIDER)
                ? new String[] {android.location.LocationManager.FUSED_PROVIDER}
                : new String[] {android.location.LocationManager.GPS_PROVIDER, android.location.LocationManager.NETWORK_PROVIDER};
        boolean any = false;
        for (String provider : ask) {
            if (!there.contains(provider) || (android.location.LocationManager.GPS_PROVIDER.equals(provider) && !fine)) {
                continue;
            }
            try {
                places.requestLocationUpdates(provider, 10000, 20, whereabouts, android.os.Looper.getMainLooper());
                // what the system knew a moment ago, so the page has something at once (an older one is not "now")
                android.location.Location last = places.getLastKnownLocation(provider);
                if (last != null && System.currentTimeMillis() - last.getTime() < 120000) {
                    whereabouts.onLocationChanged(last);
                }
                any = true;
            } catch (SecurityException | IllegalArgumentException e) {
                // (this provider is not for the app: the next one)
            }
        }
        return any;
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
                healthTimer.postDelayed(locationStep, 3000);
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
        healthTimer.removeCallbacks(locationStep);
        if (locating) {
            android.location.LocationManager places = (android.location.LocationManager) getSystemService(LOCATION_SERVICE);
            if (places != null) {
                places.removeUpdates(whereabouts);
            }
        }
        web.destroy();
        super.onDestroy();
    }
}
