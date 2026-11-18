# Privacy

octopus collects no data. It has no analytics, no crash reporting, no ads and no accounts held by us. We run no server that your data passes through or rests on.

## Where your data lives

Your tasks, terminals, files and browser panels live on the computers you run an octopus server on. The apps on your other devices reach those servers directly, peer to peer, over connections encrypted end to end. Your account is a list of your devices that each device keeps for itself; no service holds it. Each device's key stays in that device's secure storage (the Keychain on iOS).

## The relay

When two of your devices can't reach each other directly, their encrypted traffic passes through a public relay run by n0, the makers of iroh, the networking library octopus is built on. The relay forwards bytes it cannot read, and sees the IP addresses of the devices it connects. You can point your servers at a relay of your own instead.

## What the app asks to use

- **Camera:** to scan a server's pairing QR code, and to take a photo you choose to paste into a terminal. Images go only to the terminal you paste them into.
- **Microphone:** only when a program in a terminal you hold asks to hear it.
- **Local network:** to reach your servers directly on the same network.
- **Clipboard:** read only when you paste.

## Changes and contact

Changes to this page appear in this repository's history. Questions go to the [issue tracker](https://github.com/zabealbe/octopus/issues).
