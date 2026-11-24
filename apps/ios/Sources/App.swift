// The iPhone and iPad shell: one window that is the system's web view, showing the tracker page that the
// Rust side serves inside this app on 127.0.0.1. Everything the app does is in the pages and in Rust; this
// file only starts them and answers the web view's question about the camera.
import UIKit
import WebKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // a camera that is watched should not have its screen go dark; iOS stops the camera of an app that is
        // not in front, which no setting of the app can change
        application.isIdleTimerDisabled = true
        return true
    }
}

// The app's one window belongs to a scene, and so does what opens the app (Info.plist names this class as the
// scene's delegate). iOS 27 starts no app that hasn't taken up scenes: it ended this one the moment it was
// opened on an iPhone 17 (2026-11-23), where iOS 26 only said so in the log.
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession,
               options connectionOptions: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        window.rootViewController = WebController()
        window.makeKeyAndVisible()
        self.window = window
        // the app was started by a join code
        open(connectionOptions.urlContexts)
    }

    // The phone's Camera app read a join code (facetracker-join:...) and the owner chose to open it here.
    func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
        open(URLContexts)
    }

    private func open(_ contexts: Set<UIOpenURLContext>) {
        guard let url = contexts.map(\.url).first(where: { $0.scheme == "facetracker-join" }),
              let web = window?.rootViewController as? WebController else { return }
        web.openJoin(url.absoluteString)
    }
}

final class WebController: UIViewController, WKUIDelegate, WKNavigationDelegate {
    private var web: WKWebView!
    /// The app's own server, once it runs.
    private var origin: String?
    /// A join code the app was opened with, until the join page has it.
    private var joinCode: String?
    /// The loading screen, while it is shown.
    private var loading: UIView?

    /// How warm the phone says it is (its own four states; a phone gives no degrees), its battery, and whether it
    /// is on power: handed to the app as numbers.
    private static func sayHealth() {
        let heat: Int32
        switch ProcessInfo.processInfo.thermalState {
        case .nominal: heat = 0
        case .fair: heat = 1
        case .serious: heat = 2
        case .critical: heat = 3
        @unknown default: heat = -1
        }
        let level = UIDevice.current.batteryLevel             // (below 0: the system doesn't say)
        let onPower: Int32
        switch UIDevice.current.batteryState {
        case .unplugged: onPower = 0
        case .charging, .full: onPower = 1
        default: onPower = -1
        }
        face_app_health(heat, level < 0 ? -1 : level * 100, onPower)
    }

    /// The join page, asked about the code: nothing is done with a code before the owner says so there.
    func openJoin(_ code: String) {
        guard let origin, isViewLoaded,
              let asked = code.addingPercentEncoding(withAllowedCharacters: .alphanumerics),
              let url = URL(string: "\(origin)/join?code=\(asked)") else {
            joinCode = code
            return
        }
        web.load(URLRequest(url: url))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true                 // the camera's picture in the page, not full screen
        config.mediaTypesRequiringUserActionForPlayback = []    // it starts without a tap
        web = WKWebView(frame: view.bounds, configuration: config)
        web.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        web.uiDelegate = self
        web.navigationDelegate = self
        // dark until its page is there: an empty web view is white
        web.isOpaque = false
        web.backgroundColor = .black
        web.scrollView.backgroundColor = .black
        view.addSubview(web)
        showLoading()

        let bundle = Bundle.main.bundlePath
        let home = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].path
        try? FileManager.default.createDirectory(atPath: home, withIntermediateDirectories: true)
        // what the shared database's other devices see this one as, once it joins one
        face_app_device(UIDevice.current.name, "\(UIDevice.current.systemName) \(UIDevice.current.systemVersion)")
        // the phone's heat state and battery, for the Signals chart: only the system's own interfaces give them
        // (numbers and a state; nothing of it is kept)
        UIDevice.current.isBatteryMonitoringEnabled = true
        WebController.sayHealth()
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in WebController.sayHealth() }
        // The models are loaded and tried off the main thread: that takes seconds (the first start after an
        // install longer, while the system prepares the models for its graphics processor), and iOS ends an
        // app whose main thread doesn't answer for too long at its start. It did, once the phone ran warm:
        // "terminated due to signal 9" in the middle of the models' check.
        DispatchQueue.global(qos: .userInitiated).async {
            let port = face_app_start(bundle, home, bundle + "/pages")
            let why = port > 0 ? nil : (face_app_error().map { String(cString: $0) } ?? "no reason given")
            DispatchQueue.main.async { self.started(port, why) }
        }
    }

    /// The tracker runs (its page is loaded), or didn't start (why is shown).
    private func started(_ port: Int32, _ why: String?) {
        guard port > 0, let url = URL(string: "http://127.0.0.1:\(port)/") else {
            show("The tracker didn't start: \(why ?? "no reason given")")
            return
        }
        origin = "http://127.0.0.1:\(port)"
        if let code = joinCode {
            joinCode = nil
            openJoin(code)
        } else {
            web.load(URLRequest(url: url))
        }
    }

    /// The logo, the app's name and a line, over the still empty web view while the models load (seconds; the
    /// first start after an install longer). Taken away when the page has loaded.
    private func showLoading() {
        let cover = UIView(frame: view.bounds)
        cover.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        cover.backgroundColor = .black
        let logo = UIImageView(image: UIImage(named: "Logo"))
        logo.contentMode = .scaleAspectFit
        logo.layer.cornerRadius = 24
        logo.clipsToBounds = true
        let title = UILabel()
        title.text = "Face Tracker"
        title.textColor = .white
        title.font = .systemFont(ofSize: 24, weight: .semibold)
        let note = UILabel()
        note.text = "Loading the models…"
        note.textColor = UIColor(white: 0.7, alpha: 1)
        note.font = .systemFont(ofSize: 15)
        let busy = UIActivityIndicatorView(style: .medium)
        busy.color = .white
        busy.startAnimating()
        let stack = UIStackView(arrangedSubviews: [logo, title, note, busy])
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 14
        stack.setCustomSpacing(22, after: logo)
        stack.translatesAutoresizingMaskIntoConstraints = false
        cover.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: cover.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: cover.centerYAnchor),
            logo.widthAnchor.constraint(equalToConstant: 120),
            logo.heightAnchor.constraint(equalToConstant: 120),
        ])
        view.addSubview(cover)
        loading = cover
    }

    private func hideLoading() {
        guard let cover = loading else { return }
        loading = nil
        UIView.animate(withDuration: 0.25, animations: { cover.alpha = 0 }, completion: { _ in cover.removeFromSuperview() })
    }

    private func show(_ text: String) {
        hideLoading()
        let label = UILabel(frame: view.bounds.insetBy(dx: 24, dy: 80))
        label.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        label.numberOfLines = 0
        label.textColor = .white
        label.text = text
        view.addSubview(label)
        print(text)
    }

    // The page asks for the camera (and the microphone): the app's own page, served by the app itself, so
    // the answer is yes. iOS still asks the owner once, for the app as a whole.
    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
                 initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
                 decisionHandler: @escaping (WKPermissionDecision) -> Void) {
        let own = origin.host == "127.0.0.1" && frame.isMainFrame
        print("Camera asked for by \(origin.host):\(origin.port): \(own ? "granted" : "left to the system's question")")
        decisionHandler(own ? .grant : .prompt)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        print("The page failed: \(error.localizedDescription)")
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        print("The page didn't load: \(error.localizedDescription)")
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        hideLoading()
        // what the page itself can say about the camera: is this a place a page may use one at all?
        webView.evaluateJavaScript("[window.isSecureContext, !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia)].join(',')") { value, _ in
            print("Page loaded; secure context, camera available to pages: \(value ?? "?")")
        }
    }
}
