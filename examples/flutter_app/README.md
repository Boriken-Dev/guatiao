# flutter_app

guatiao in a Flutter app. The three Dart packages come from this checkout,
and their build hooks build the libraries with cargo for the target being
built: nothing to install but rustup, the Rust targets of the platforms
you build for, and the platform's usual Flutter setup.

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android   # for Android
flutter build apk          # or appbundle, ios, linux, macos, windows
```

At start the app builds a value, writes it as JSON, reads it back, finds
a field by its path, and prints where each library was loaded from (the
`flutter` tag of Android's log, the console on a desktop). With
`GUATIAO_EXAMPLE_EXIT=1` a desktop run ends there, with exit status 0
when every call worked.

`pubspec.yaml` shows the user define that builds `../app_library`, an
application's own library carrying every surface, instead of the three.
The docs site's "Dart and Flutter" page is the whole setup.
