#!/bin/sh
# On the Mac: write the Xcode project for the iPhone app (Sources/App.swift around the Rust library).
# The sources are expected in ~/fr-test/rust-src (this repository's rust/ folder), the model runtime's
# xcframework in ~/fr-test/ort-ios/x, and the first line of ~/fr-test/team.txt is the developer team's id.
# The app's id is its permanent one, the same as the Android app's and the Mac app's (FACE_IOS_BUNDLE names
# another, for a copy beside it); its version is the workspace's (Cargo.toml: 2.8.0 is build 20800, as on
# Android). It is an iPhone app (an iPad runs it as one): for the iPad's own layout the App Store wants every
# way up, and its own screenshots.
# The Rust library is linked by its full path: asked for by name, the linker took the dynamic one cargo builds
# beside it, which is not in the app (the app then ended the moment it was opened).
set -e
TEAM=$(sed -n 1p ~/fr-test/team.txt)
BUNDLE=${FACE_IOS_BUNDLE:-io.github.singularityresearch.facerecognizer}
RUST=$HOME/fr-test/rust-src
SRC=$RUST/apps/ios
VERSION=$(sed -n 's/^version = "\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/p' "$RUST/Cargo.toml" | head -1)
BUILD=$(echo "$VERSION" | awk -F. '{ print $1 * 10000 + $2 * 100 + $3 }')
[ -n "$VERSION" ] || { echo "no version in $RUST/Cargo.toml"; exit 1; }
XCFW=$HOME/fr-test/ort-ios/x/onnxruntime.xcframework
P=$HOME/fr-test/iosapp
rm -rf "$P"; mkdir -p "$P/FaceApp.xcodeproj"
cat > "$P/FaceApp.xcodeproj/project.pbxproj" <<EOF
// !\$*UTF8*\$!
{
	archiveVersion = 1;
	classes = {};
	objectVersion = 56;
	objects = {
		B00000000000000000000001 /* App.swift in Sources */ = {isa = PBXBuildFile; fileRef = B00000000000000000000002; };
		B00000000000000000000002 /* App.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; name = App.swift; path = "$SRC/Sources/App.swift"; sourceTree = "<absolute>"; };
		B00000000000000000000003 /* FaceApp.app */ = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = FaceApp.app; sourceTree = BUILT_PRODUCTS_DIR; };
		B00000000000000000000004 = {isa = PBXGroup; children = (B00000000000000000000002, B0000000000000000000000E, B00000000000000000000011, B00000000000000000000006, ); sourceTree = "<group>"; };
		B00000000000000000000006 /* Products */ = {isa = PBXGroup; children = (B00000000000000000000003, ); name = Products; sourceTree = "<group>"; };
		B00000000000000000000007 /* Sources */ = {isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = (B00000000000000000000001, ); runOnlyForDeploymentPostprocessing = 0; };
		B00000000000000000000008 /* FaceApp */ = {isa = PBXNativeTarget; buildConfigurationList = B0000000000000000000000B; buildPhases = (B00000000000000000000007, B00000000000000000000010, ); buildRules = (); dependencies = (); name = FaceApp; productName = FaceApp; productReference = B00000000000000000000003; productType = "com.apple.product-type.application"; };
		B00000000000000000000009 /* Project object */ = {isa = PBXProject; attributes = {LastUpgradeCheck = 2700; TargetAttributes = {B00000000000000000000008 = {ProvisioningStyle = Automatic; }; }; }; buildConfigurationList = B0000000000000000000000A; compatibilityVersion = "Xcode 14.0"; developmentRegion = en; hasScannedForEncodings = 0; knownRegions = (en, Base, ); mainGroup = B00000000000000000000004; productRefGroup = B00000000000000000000006; projectDirPath = ""; projectRoot = ""; targets = (B00000000000000000000008, ); };
		B0000000000000000000000A /* Build configuration list for PBXProject */ = {isa = XCConfigurationList; buildConfigurations = (B0000000000000000000000C, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release; };
		B0000000000000000000000B /* Build configuration list for PBXNativeTarget */ = {isa = XCConfigurationList; buildConfigurations = (B0000000000000000000000D, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release; };
		B0000000000000000000000C /* Release */ = {isa = XCBuildConfiguration; buildSettings = {SDKROOT = iphoneos; IPHONEOS_DEPLOYMENT_TARGET = 16.0; ARCHS = arm64; }; name = Release; };
		B0000000000000000000000E /* Assets.xcassets */ = {isa = PBXFileReference; lastKnownFileType = folder.assetcatalog; name = Assets.xcassets; path = "$SRC/Assets.xcassets"; sourceTree = "<absolute>"; };
		B0000000000000000000000F /* Assets.xcassets in Resources */ = {isa = PBXBuildFile; fileRef = B0000000000000000000000E; };
		B00000000000000000000011 /* PrivacyInfo.xcprivacy */ = {isa = PBXFileReference; lastKnownFileType = text.xml; name = PrivacyInfo.xcprivacy; path = "$SRC/PrivacyInfo.xcprivacy"; sourceTree = "<absolute>"; };
		B00000000000000000000012 /* PrivacyInfo.xcprivacy in Resources */ = {isa = PBXBuildFile; fileRef = B00000000000000000000011; };
		B00000000000000000000010 /* Resources */ = {isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = (B0000000000000000000000F, B00000000000000000000012, ); runOnlyForDeploymentPostprocessing = 0; };
		B0000000000000000000000D /* Release */ = {isa = XCBuildConfiguration; buildSettings = {
			PRODUCT_NAME = FaceApp;
			PRODUCT_BUNDLE_IDENTIFIER = "$BUNDLE";
			CODE_SIGN_STYLE = Automatic;
			DEVELOPMENT_TEAM = $TEAM;
			GENERATE_INFOPLIST_FILE = YES;
			INFOPLIST_FILE = "$SRC/Info.plist";
			INFOPLIST_KEY_CFBundleDisplayName = "Face Recognizer";
			INFOPLIST_KEY_NSCameraUsageDescription = "The camera's picture is read on this device to find and name the faces in it.";
			INFOPLIST_KEY_NSMicrophoneUsageDescription = "The microphone is listened to on this device for voice signals and for the Signals chart's sound, when you switch those on. Nothing is recorded.";
			INFOPLIST_KEY_NSLocalNetworkUsageDescription = "Your shared database is on your own network: the names come from there.";
			INFOPLIST_KEY_NSLocationWhenInUseUsageDescription = "Shows where this device is, to you and your other devices, and keeps its moves in your shared database, while you have switched that on.";
			INFOPLIST_KEY_UILaunchScreen_Generation = YES;
			INFOPLIST_KEY_UISupportedInterfaceOrientations = "UIInterfaceOrientationPortrait UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
			ASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;
			TARGETED_DEVICE_FAMILY = 1;
			SWIFT_VERSION = 5.0;
			SWIFT_OBJC_BRIDGING_HEADER = "$SRC/Sources/face_app.h";
			FRAMEWORK_SEARCH_PATHS = "$XCFW/ios-arm64";
			OTHER_LDFLAGS = "$RUST/target/aarch64-apple-ios/release/libface_app.a -framework onnxruntime -lc++ -framework CoreML -framework Security -liconv -lresolv -lsqlite3 -lz -framework Network";
			CURRENT_PROJECT_VERSION = $BUILD;
			MARKETING_VERSION = $VERSION;
			SDKROOT = iphoneos;
			IPHONEOS_DEPLOYMENT_TARGET = 16.0;
		}; name = Release; };
	};
	rootObject = B00000000000000000000009 /* Project object */;
}
EOF
echo "project written: $P (version $VERSION, build $BUILD)"
