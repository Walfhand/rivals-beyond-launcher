# Privacy policy

The Rivals Beyond launcher sends bounded technical diagnostics and game-launch/exit events to Sentry
by default. The settings checkbox **Automatically send technical diagnostics** disables
delivery for games started with it unchecked, and that preference is remembered. Records from those
games are never uploaded on a later opt-in. There is no advertising or persistent player/device tracking.

It makes HTTPS requests to the Rivals Beyond object-storage service to:

- check signed launcher and game-client manifests;
- download files when the user installs, updates or repairs the client;
- download a launcher update when a newer signed version is available.

It also requests public articles from `https://api.rivalsbeyond.com/api/v1/news`, sending the
launcher language (`fr` or `en`) and pagination parameters. No account credentials are sent.

Network requests necessarily expose the user's IP address and standard HTTP metadata to the receiving
storage, website API or Sentry service. Sentry receives technical error text, bounded stack/context information,
client/launcher versions, graphics mode, module/patch manifest hashes, a new random session identifier
per launch, start/exit events, duration and process exit status. This measures launches, not unique players.

The game writes a bounded local diagnostic journal. The launcher reads only that journal and warning/error
lines from the current WarcraftXL log; it does not scan arbitrary log folders. Account files, chat,
screenshots, game saves, memory dumps and the installation path are not collected. Emails, URLs,
credential-like tokens and local user paths in error text are scrubbed before upload. Error text is
unstructured, so this filtering is not a guarantee that every possible incidental personal value can
be recognized. The service uses the project's Sentry endpoint in the DE ingestion region; retained
server-side data follows the configured Sentry plan and project settings.

The local journal retains at most two 256 KiB files. Acknowledged event IDs and retry/quota bookkeeping
are stored locally so delivery can resume after a connection failure. Transmission runs in the background
and never blocks the game. Closing the launcher can interrupt delivery; unsent records may be retried
on a later enabled launch or expire when the bounded journal rotates.

The selected client directory, language preferences and the last valid API news response for each language are stored locally. The launcher starts the locally installed game executable only after the user chooses **Play**.

Account creation and news links open `rivalsbeyond.com` in the system browser only after an explicit
click. The website's own privacy policy then applies.
