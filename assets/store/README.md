# The stores' pictures

What the listings in `docs/store/` show: the app's icon in the shapes the stores ask for, and screenshots.

- `google-play-icon-512.png`, `google-play-feature-1024x500.png`, `microsoft-store-boxart-1080.png`,
  `microsoft-store-poster-720x1080.png`: the app's one drawing (`assets/icons`), drawn again at these shapes by the
  code that drew the icon. The App Store's picture is the iPhone app's icon (`apps/ios/Assets.xcassets`).
- `screenshots/windows` (1920 × 1080), `screenshots/iphone` (1320 × 2868), `screenshots/android` (1080 × 1920):
  the app itself, run as a test copy with a library of its own, at each screen's size.

## The people in the screenshots do not exist

Every face in them was generated from a written description (Stable Diffusion XL base 1.0, whose licence, CreativeML
Open RAIL++-M, lets its pictures be used commercially), and every name is invented: Maya Torres, Daniel Okafor,
Priya Nair, Tom Becker, Hana Kim, Omar Haddad, Grace Whitfield. Nobody's photo, likeness or name was used; a
likeness to a real person, or a real person of one of these names, is chance.

What the app shows about them is real: the test copy's library was filled with generated photos of each made-up
person through the app's own **Add photos**, the faces were named through its own labeling, and the camera was a
video of another generated picture of the same face. The names, the match percentages, the expression and the
other readings are what the app worked out. One thing was changed for the pictures: the camera's name in the list
(a made-up camera is a video file, and its name was that file's path).

When a screenshot is made again, make it the same way: never with a real person's picture.
