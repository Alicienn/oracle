# Changelog

Every released version has a section here, and the release script refuses to publish one
that does not: an update that cannot say what it changed is an update nobody has a reason to
accept. The section for the running version is what Oracle shows after it updates itself.

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
