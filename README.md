## What is this?

Just an overengineered solution to my laziness.
It basically does this:

- Calls `yt-dlp`
- Does some fancy fingerprinting stuff
- Calls `beet import`

More details below

## How to build

Pretty much like any other rust project: clone it, inspect the code, `cargo build` it.

## But how do I use it?

That's more interesting.

IMPORTANT: unless `yt-dlp`, `ffmpeg` and `fpcalc` (chromaprint) exist on the `PATH` with which `yt-dlp-wrapper` was
invoked with, you'll need to specify the full path to these executables with the appropriate cli parameters.

This project is separated into two logical parts.
The `tty` (or `daemon`) instance and the `request` instances.

There can only be one `tty` instance (it checks with a lockfile in `/tmp`).
Its job is to sit in your terminal ready to receive "Video Requests" from `request` instances (it does so via a
`127.0.0.1` http server, with a random port).
Once it does, it will begin the yt-dlp → fingerprint → beet process (sequentially, more below).

`request` instances, on the other hand, take in a `--yt-url`, extract its video ID, and send it over to the `tty` (if
it's running).

Some things can be configured, run `yt-dlp-wrapper tty --help` and `yt-dlp-wrapper request --help` for more information
on those.

## Contributing
You're welcome to open an issue or even a PR!

But do note that this is just a little project I made for fun.

## TTY Instance

At the bare minimum, the `tty` instance will require a `--yt-dlp` parameter, which represents the command that will get
invoked to download the videos.

NOTE: I had `yt-dlp` in mind when creating this, but as long as the command accepts `--` and `<youtube-id>` as the last
two parameters, and downloads the audio files
in the current working directory (which will be set to a random `/tmp` dir), it will work.

The `tty` instance will split the command string using [shlex](https://crates.io/crates/shlex), which will only catch
very basic syntax errors.

You can invoke it directly from your terminal or set up a keybind/script with all your preferred parameters.

The `tty` instance tries to handle CTRL-C signals gracefully.
If you just want to kill the process **_right now_**, hitting CTRL-C exactly two or three times (it depends on how fast
you are) should kill it instantly, unless the program is stuck.
In that case, you might have to kill the terminal.

### Download Process

Requests will be handled one at a time, sequentially, but can be received at any time, and will get enqueued up to a
maximum of `--max-requests` (`--help` for defaults).

When a Video Request is received, the program will:

1. Create a new `/tmp` directory which will get deleted when the Video Request is done (see `--keep-tmp`)
2. Execute `<yt-dlp-command> -- <youtube-id>`
3. Ask what files you would like to fingerprint (`yt-dlp` can download multiple files based on the configuration)
    - Fingerprint them with `fpcalc`
4. For each fingerprinted file:
    1. Lookup the fingerprint on `https://acoustid.org`, getting the bound musicbrainz information (if any)
    2. Ask the user to select the correct musicbrainz recording id which represents the audio file (or ask if the user
       wants to submit the fingerprint: see below).
    3. Use `ffmpeg` to modify the audio file metadata to include the `MusicBrainz Track Id`, overwriting the `Title` and
       `Artist` metadata to match the musicbrainz recording (this helps `beet` import the file later)
5. Execute `<beet-import-cmd> .` in the `/tmp` directory.

### Fingerprint submission

IMPORTANT: As the AcoustID User API KEY can be considered sensitive information, the program will ask for it at runtime
and will only store it in RAM (which means it will ask for it again every time the `tty` instance is restarted).
Thought this might change in the future.

AcoustID fingerprint submission requires an AcoustID User API KEY (you get one when you create an account),
and a musicbrainz RECORDING id.
While technically you don't need a musicbrainz id, this program is supposed to help with beets, which works off musicbrainz.

All of this means that if you don't have an AcoustID account, you won't be able to submit fingerprints, and if there is no
musicbrainz recording matching the audio file you want to fingerprint, _you'll have to create one_.

Other than that, the program will also query the AcoustID server to let you know if your submission went well.

## Request Instance

The `request` instances do the _bare minimum_ necessary, which is to parse the youtube url and send the video id.

You _can_ invoke `request` instances from a terminal (they'll just return almost instantly), but I **highly** recommend
setting up a keybind that reads the
clipboard, passing them to `--yt-url`.

Passing bogus data to it as `--yt-url` will at worst crash the request instance, which won't affect the `tty`
instance.

## License

This project is licensed under the terms of both the MIT license and the Apache License (Version 2.0).

See [LICENSE.Apache-2.0](LICENSE.Apache-2.0), [LICENSE.MIT](LICENSE.MIT), and [COPYRIGHT](COPYRIGHT) for details.
