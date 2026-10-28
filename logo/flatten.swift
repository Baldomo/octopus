import CoreGraphics
import Foundation
import ImageIO

for path in CommandLine.arguments.dropFirst() {
    let url = URL(fileURLWithPath: path) as CFURL
    guard let source = CGImageSourceCreateWithURL(url, nil), let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
        fatalError("read \(path)")
    }
    let canvas = CGContext(
        data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    canvas.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard let destination = CGImageDestinationCreateWithURL(url, "public.png" as CFString, 1, nil) else {
        fatalError("write \(path)")
    }
    CGImageDestinationAddImage(destination, canvas.makeImage()!, nil)
    guard CGImageDestinationFinalize(destination) else { fatalError("write \(path)") }
}
