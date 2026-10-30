import CoreGraphics
import Foundation
import ImageIO
import SwiftUI

// Apple's macOS icon grid: a 1024 canvas holding a centered 824 continuous-corner rounded rect.
let (art, output) = (CommandLine.arguments[1], CommandLine.arguments[2])
let size = 512.0
let grid = size / 1024
let tile = CGRect(x: 100 * grid, y: 100 * grid, width: 824 * grid, height: 824 * grid)
let shape = RoundedRectangle(cornerRadius: 185.4 * grid, style: .continuous).path(in: tile).cgPath

guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: art) as CFURL, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else { fatalError("read \(art)") }
let canvas = CGContext(
    data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
canvas.interpolationQuality = .high

canvas.saveGState()
canvas.setShadow(offset: CGSize(width: 0, height: -12 * grid), blur: 28 * grid, color: CGColor(gray: 0, alpha: 0.3))
canvas.addPath(shape)
canvas.fillPath()
canvas.restoreGState()

canvas.addPath(shape)
canvas.clip()
canvas.draw(image, in: tile)

guard let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath: output) as CFURL, "public.png" as CFString, 1, nil)
else { fatalError("write \(output)") }
CGImageDestinationAddImage(destination, canvas.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("write \(output)") }
