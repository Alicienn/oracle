# Changelog

Every released version has a section here, and the release script refuses to publish one
that does not: an update that cannot say what it changed is an update nobody has a reason to
accept. The section for the running version is what Oracle shows after it updates itself.

## 0.3.1

- The embedded web app panel actually renders. In 0.3.0 it did not: the page loaded, a
  renderer ran, and the panel stayed black. Multiple webviews in one window are meant to
  *partition* it, and Oracle's own webview already covers the whole client area, so the child
  landed behind it. The web app now gets its own undecorated window, owned by the main one —
  always above its owner — kept over the panel as the window moves, resizes, is minimised or
  goes to the tray.
- A project started on an alternative port after a collision is opened at the port it is
  actually serving. Both the in-app panel and the browser button used to send you to the port
  the project was refused.
- A project that is running but answering nothing says so, instead of handing over
  WebView2's own "cannot reach this page" — and names the usual cause, which is a project
  serving a different port than the one in its settings.
- Opening a web app is verified rather than assumed: if the window does not appear, that is
  reported. An empty panel that says nothing is how the 0.3.0 fault hid.
- Ctrl+Shift+I opens the interface's own developer tools. Oracle is a tool for people who run
  dev servers, and when its own window misbehaved there was no way to reach a console in a
  packaged build.
- Correction to 0.3.0's note on permissions: scoping them to webviews is defence in depth,
  not a hole closed. A remote origin has no IPC access to begin with unless a capability
  explicitly grants it.

## 0.3.0

- Project web apps open inside Oracle. Pressing start on a project that serves a page shows
  it booting in the central panel, waits a beat past the moment its port answers — a server
  that is listening is often still compiling — and then reveals the page. The rail and the
  title bar stay where they are.
- Leaving a web app keeps it alive off screen, so coming back does not reload the page or
  lose what was typed into it. The strip above it says what that costs in memory, and closing
  the view gives it back.
- The rail can be expanded to show project names, and collapsed again. It starts with a
  Dashboard entry, separated from the projects, which is how you get back out of a web app.
- The interface reports its own faults. One `render` call rebuilds every region, so an
  exception anywhere inside it used to abandon the rest and leave the window half-drawn with
  nothing said — which is how the web app panel first appeared as an empty black rectangle.
  Each region is now guarded and named, and anything that escapes becomes a toast with its
  stack.
- Settings has a Changelog button, next to the update controls.
- A project's icon can be fetched again, past the cache, from its form.
- Security: Oracle's permissions are now scoped to its own webviews rather than to the window
  holding them. A capability granted to a window is granted to every webview inside it, which
  would have handed an embedded project page the whole command surface.

## 0.2.1

- Fixed: "Check for updates" in Settings found an update and then closed the dialog without
  saying so, which was indistinguishable from a button that does nothing. It now reports what
  it found in the row that was clicked.

## 0.2.0

- Oracle updates itself. An available update appears as a line in the title bar rather than
  a dialog over what you were doing; installing is a separate step that stops your running
  projects first, so nothing is left orphaned holding a port.
- Two projects can no longer start on the same port. Oracle refuses before spawning
  anything, names what is holding the port, and offers the next free one for that run.
- A running project is readable at a glance: its card carries an accent edge, so the answer
  to "what is up right now" does not need reading eight status dots.
- Start and stop show progress. The button spins until the backend answers, and a start that
  is going nowhere can still be abandoned.
- Changing the filter animates. Projects that both filters admit slide to their new row
  instead of being rebuilt in place.
- Projects reachable over HTTP are drawn with the icon their site serves.
- Projects can be reordered by dragging their cards, not only their rail icons.
- The tray panel got the component styles it had never loaded: its buttons, status dots and
  toasts were unstyled, and its search field was breaking the window's layout.
- Resident memory is down by around 120 MB. The tray panel's webview is built when it is
  first opened instead of living hidden for the whole session, and log buffers are no longer
  kept for projects nobody is looking at.
- Fixed: a project serving on IPv6 loopback was reported as not responding. A project that
  had crashed could not be restarted. An unhealthy project could not be stopped. Clicking a
  card's play button also selected the card.

## 0.1.0

- First build: launch and stop local projects, watch their CPU and memory, check remote
  deployments over HTTP, read git status, and reach all of it from a tray panel.
