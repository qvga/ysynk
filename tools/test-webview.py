#!/usr/bin/env python3
"""Exercise the built UI with fixture data in the same web engine as the Linux app."""
from pathlib import Path
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2


def run(fixture, width, height):
    window = Gtk.Window()
    window.set_default_size(width, height)
    view = WebKit2.WebView.new_with_context(WebKit2.WebContext.new_ephemeral())
    view.get_settings().set_enable_write_console_messages_to_stdout(True)
    window.add(view)
    errors = []

    def finish(webview, result):
        try:
            viewport = webview.call_async_javascript_function_finish(result).to_string()
            if viewport != f"{width}×{height}":
                errors.append(f"Expected {width}×{height}, got {viewport}")
        except GLib.Error as exc:
            errors.append(exc.message)
        Gtk.main_quit()

    def loaded(webview, event):
        if event == WebKit2.LoadEvent.FINISHED:
            webview.call_async_javascript_function(
                """
                // WebKit can report FINISHED before the inline ES module runs.
                while (typeof window.verifyAuditUi !== "function")
                    await new Promise(requestAnimationFrame);
                const result = await window.verifyAuditUi();
                return result.viewport.join("×");
                """, -1, None, None, None, None, finish
            )

    def timeout():
        errors.append("Timed out waiting for UI checks")
        Gtk.main_quit()
        return GLib.SOURCE_REMOVE

    view.connect("load-changed", loaded)
    window.show_all()
    timer = GLib.timeout_add_seconds(90, timeout)
    view.load_uri(fixture.as_uri())
    Gtk.main()
    if GLib.MainContext.default().find_source_by_id(timer):
        GLib.source_remove(timer)
    window.destroy()
    if errors:
        raise RuntimeError(f"{width}×{height}: {errors[0]}")
    print(f"WebKitGTK UI checks passed at {width}×{height}", flush=True)


if __name__ == "__main__":
    fixture = Path(__file__).resolve().parents[1] / "target/audit-ui/index.html"
    if not fixture.is_file():
        sys.exit("Build the frontend and run node ui/audit-fixture.mjs first.")
    for size in [(1120, 800), (720, 580)]:
        run(fixture, *size)
