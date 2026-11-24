package io.github.singularityresearch.facerecognizer;

/** The Rust side of the app (crates/face-app, libface_app.so). */
final class Native {
    static {
        System.loadLibrary("face_app");
    }

    /** Start the live tracker inside this app; its port on 127.0.0.1, or -1 (error() says why). */
    static native int start(String root, String home, String pages, String runtime);

    static native String error();

    /**
     * The phone's heat state (0 nominal, 1 fair, 2 serious, 3 critical; -1 not known), its battery in percent
     * (-1 not known) and whether it is on power (0, 1; -1 not known): said every few seconds, for the Signals
     * chart.
     */
    static native void health(int heat, float battery, int onPower);

    /** What the phone is called and its system ("Android 16"), before start(). */
    static native void device(String name, String os);

    private Native() {
    }
}
