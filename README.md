## What is this?

Just an overengineered solution to my laziness.
It basically does this:

- Calls `yt-dlp`
- Does some fancy fingerprinting stuff
- Calls `beet import`

### Important notes

This program requires [yt-dlp](https://github.com/yt-dlp/yt-dlp) (or compatible alternative,
see [below](#tty-instance)),
[ffmpeg](https://ffmpeg.org/) and [fpcalc](https://acoustid.org/chromaprint).

If `yt-dlp`, `ffmpeg` and `fpcalc` don't exist on the `PATH` with which `yt-dlp-wrapper` was
invoked, you'll need to specify the full path to these executables with the appropriate cli parameters.
This might happen when you invoke `yt-dlp-wrapper` through a DE executor (e.g., through a keybind).

## How to build

Pretty much like any other rust project: clone the repo, inspect the code, `cargo build` the project.

I included a utility [deploy-to-local](deploy-to-local) to copy the compiled executable
and [yt-dlp-wrapper-submit-request](yt-dlp-wrapper-submit-request) to `~/.local/bin/`.

If you want to copy only the executable or change the destination, after `cargo build`-ing the project, you'll find the
executables in `./taget/{debug,release}/yt-dlp-wrapper`.

I've compiled and used this program on my linux distro, but there shouldn't be
anything actively stopping this program from running on Windows.
I _might_ test it in the future.

## But how do I use it?

This project is separated into two logical parts.
The [tty](#tty-instance) instance and the [request](#request-instance) instances.

There can only be one `tty` instance (unless you explicitly disable the lockfile).
Its job is to sit in your terminal, ready to receive "Video Requests" from `request` instances,
via a [local](https://en.wikipedia.org/wiki/Localhost) http server, with a random or given port.
Once it does, it will begin the yt-dlp → fingerprint → beet process (sequentially, more below).

`request` instances, on the other hand, take in a `--yt-url`, extract its video ID, and send it over to the `tty`
(if it's running).

Quite a bit of things can be configured, run `yt-dlp-wrapper tty --help` and `yt-dlp-wrapper request --help` for more
information.

### Why two parts?

The main reason for _not_ just asking for the url in the terminal is to allow for a keybind to be pressed
from any window.
That way, you can copy/enqueue videos in batches instead of switching from one window to the other.

## TTY Instance

The `tty` instance _can_ run without any additional parameters, but I recommend setting up `yt-dlp`'s
parameters through `--yt-dlp-args`.

NOTE: I had `yt-dlp` in mind when making this project, but as long as the program you want to use can be invoked as
`<the-executable> <the-args> -- <video-id>`, and downloads the audio/video files
in the current working directory, which will be set to a random `/tmp` dir for the whole process, **it should work**.

The `tty` instance **will split** the args string using [shlex](https://crates.io/crates/shlex), which means you can
safely pass multiple space-separated arguments in the `*-args` parameters like so:

```shell
yt-dlp-wrapper tty --yt-dlp-args '--no-playlist --embed-thumbnail --format 140'
```

The `tty` instance tries to handle CTRL-C signals gracefully, but if CTRL-C is pressed for a second time and the handler
didn't show up, it will just exit.

In case you need to kill the process for any reason, the `pid` is the first line in
`/tmp/a81f7509-2019-4fb9-8d72-ba66c897df34.lock` (unless you disabled the lockfile functionality).

### Download Process

Requests will be handled one at a time, sequentially, but can be received at any time, and will get enqueued up to a
maximum of `--max-requests` (`--help` for defaults).

When a Video Request is received, the program will:

1. Create a new `/tmp` directory which will get deleted when the Video Request is done (see `--keep-tmp`)
2. Execute `<yt-dlp> <yt-dlp-args> -- <youtube-id>`
3. Ask what files you would like to fingerprint (`yt-dlp` can download multiple files based on the configuration)
    - Fingerprint them with `<fpcalc>`
4. For each fingerprinted file:
    1. Lookup the fingerprint on `https://acoustid.org`, getting the bound musicbrainz information (if any)
    2. Ask the user to select the correct musicbrainz recording id which represents the audio file
        - If none: ask the user if they want to [submit the fingerprint](#fingerprint-submission).
    3. Use `<ffmpeg>` to modify the audio file metadata to include the `MusicBrainz Track Id`, `Title` and
       `Artist` metadata to match the musicbrainz recording (this helps `beet` import the file later)
5. Execute `<beet> <beet-args> .` in the `/tmp` directory.

### Fingerprint submission

Submitting a fingerprint through this project requires an AcoustID User API KEY
(you get one when you create an account),
and a musicbrainz RECORDING id.
While technically you don't need a musicbrainz id for a pure fingerprint submission,
this program is supposed to help with beets, which works off musicbrainz.

This means that, if there is no musicbrainz recording matching the audio track
you want to fingerprint, _you'll have to create one_.

Other than that, the program will also query the AcoustID server to let you know if your submission went well.

**IMPORTANT**: As the AcoustID User API KEY can be considered sensitive information, the program will ask for it at
runtime and will only store it in RAM (which means it will ask for it again every time the `tty` instance is restarted).
Thought this might change in the future if requested.

## Request Instance

The `request` instances just try to parse the YouTube url and send the video id to the `tty` instance.

You _can_ invoke `request` instances from a terminal (they'll just return almost instantly), but I **highly** recommend
setting up a keybind that reads the clipboard contents, passing them to `--yt-url`.

I recommend reading (or using) [yt-dlp-wrapper-submit-request](yt-dlp-wrapper-submit-request) for a couple nice-to-have

Passing bogus data to it as `--yt-url` will at the very worst crash the request instance, which won't affect the `tty`
instance.

## Contributing

You're welcome to open an issue or a PR!

**NOTE:** if the feature you wanted to implement through a PR is in the [Future features](#future-features) section,
_please_ open an issue first, as I might already be working on it.

But do note that this is just a little project I made for fun (and laziness).

## Future features

I may or may not implement:

- Create a "queue" file on disk (configure through args)
    - Allow enqueued requests that haven't been handled to be saved to this file (to save progress)
    - Watch the file to allow external modifications

## License

This project is licensed under the terms of both the MIT license and the Apache License (Version 2.0).

See [LICENSE.Apache-2.0](LICENSE.Apache-2.0), [LICENSE.MIT](LICENSE.MIT), and [COPYRIGHT](COPYRIGHT) for details.
