# ML Kit's component discovery instantiates each registrar named in the manifest through its no-arg constructor,
# which R8 full mode strips under firebase-components' own `-keep class * implements ComponentRegistrar`.
-keep class * implements com.google.firebase.components.ComponentRegistrar {
  <init>();
}
