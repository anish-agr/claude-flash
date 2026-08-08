// ClaudeFlash - a full-screen tinted flash that tells you Claude Code needs you.
//
// Green  = Claude finished responding   (Stop)
// Blue   = Claude has a question        (PreToolUse / AskUserQuestion)
// Violet = Claude wants to run something (PreToolUse / tool names, opt-in)
//
// The overlay is a click-through layered window: it never steals focus and never
// swallows a click. Any mouse button or key press dismisses it instantly.
//
// Targets C# 5 / .NET Framework 4.x so it builds with the csc.exe that ships
// with Windows - no SDK required.

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace ClaudeFlash
{
    internal static class Program
    {
        private const string Usage =
            "ClaudeFlash\r\n\r\n" +
            "  flash                 green flash (Claude is done)\r\n" +
            "  flash done            green flash\r\n" +
            "  flash ask             blue flash (Claude has a question)\r\n" +
            "  flash perm            violet flash (Claude wants to run something)\r\n" +
            "  flash <color>         green | blue | violet | amber | red | lavender | indigo\r\n" +
            "                        | teal | pink | purple | cyan | white | #RRGGBB\r\n\r\n" +
            "  flash set <key> <val> change a setting, e.g. flash set color_ask #08A9FF\r\n" +
            "  flash config          open config.ini\r\n" +
            "  flash reset           restore default settings\r\n\r\n" +
            "  flash on              enable flashing\r\n" +
            "  flash off             disable flashing (hooks stay installed, they just do nothing)\r\n" +
            "  flash toggle          flip enabled/disabled - green confirm = on, red confirm = off\r\n" +
            "  flash status          show current state\r\n" +
            "  flash help            this message\r\n\r\n" +
            "Options:\r\n" +
            "  --bg                  relaunch detached and return immediately (used by the hooks)\r\n" +
            "  --alpha=0.28          peak opacity, 0..1\r\n" +
            "  --fade_in_ms=70       --hold_ms=420   --fade_out_ms=560\r\n" +
            "  --vignette=0.32       how much lighter the center is than the edges\r\n\r\n" +
            "Defaults live in %LOCALAPPDATA%\\ClaudeFlash\\config.ini";

        private static Mutex _instanceLock;

        [STAThread]
        private static int Main(string[] argv)
        {
            // Hooks run this with no console attached, so an unhandled exception would
            // vanish without a trace. Leave a breadcrumb instead.
            try { return Start(argv); }
            catch (Exception ex) { LogError(ex); return 1; }
        }

        private static void LogError(Exception ex)
        {
            try
            {
                File.AppendAllText(Path.Combine(DataDir(), "error.log"),
                    DateTime.Now.ToString("u", CultureInfo.InvariantCulture) + "  " + ex + Environment.NewLine + Environment.NewLine);
            }
            catch { }
        }

        // Every branch below hands off to a [MethodImpl(NoInlining)] helper on purpose.
        // The JIT resolves a method's type references when it compiles that method, so
        // naming a WinForms type here would drag System.Windows.Forms and System.Drawing
        // into the --bg relaunch - the one path that has to be fast, and the one path
        // that draws nothing at all.
        private static int Start(string[] argv)
        {
            string mode = "done";
            bool background = false;
            var overrides = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            var positional = new List<string>();

            foreach (string raw in argv)
            {
                string arg = (raw ?? "").Trim();
                if (arg.Length == 0) continue;

                if (arg.StartsWith("--", StringComparison.Ordinal) || arg.StartsWith("/", StringComparison.Ordinal))
                {
                    string body = arg.TrimStart('-', '/');
                    int eq = body.IndexOf('=');
                    string key = eq < 0 ? body : body.Substring(0, eq);
                    string val = eq < 0 ? "" : body.Substring(eq + 1);

                    if (key.Equals("bg", StringComparison.OrdinalIgnoreCase)) background = true;
                    else if (key.Equals("help", StringComparison.OrdinalIgnoreCase) || key == "?") mode = "help";
                    else overrides[key] = val;
                }
                else
                {
                    positional.Add(arg);
                }
            }
            if (positional.Count > 0) mode = positional[0];

            string command = mode.ToLowerInvariant();
            if (command == "help" || command == "?" || command == "status" ||
                command == "on" || command == "off" || command == "toggle" ||
                command == "set" || command == "config" || command == "reset")
                return HandleCommand(command, overrides, positional);

            if (!IsEnabled()) return 0;

            if (background)
            {
                Relaunch(argv);
                return 0;
            }

            return Flash(mode, overrides);
        }

        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int HandleCommand(string command, Dictionary<string, string> overrides, List<string> positional)
        {
            EnableDpiAwareness();
            Settings settings = Settings.Load(overrides);

            switch (command)
            {
                case "set":
                    return SetConfig(positional);

                case "config":
                    try { Process.Start(Settings.ConfigPath()); }
                    catch { MessageBox.Show(Settings.ConfigPath(), "ClaudeFlash config"); }
                    return 0;

                case "reset":
                    try { File.Delete(Settings.ConfigPath()); }
                    catch { }
                    Settings.Load(null);   // writes a fresh file with the defaults
                    Run(ParseColor("green", Color.Green), Settings.Load(null));
                    return 0;

                case "help":
                case "?":
                    MessageBox.Show(Usage, "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Information);
                    return 0;

                case "status":
                    ShowStatus(settings);
                    return 0;

                case "on":
                    SetEnabled(true);
                    Confirm(true, settings);
                    return 0;

                case "off":
                    SetEnabled(false);
                    Confirm(false, settings);
                    return 0;

                case "toggle":
                    bool nowEnabled = !IsEnabled();
                    SetEnabled(nowEnabled);
                    Confirm(nowEnabled, settings);
                    return 0;
            }
            return 0;
        }

        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int Flash(string mode, Dictionary<string, string> overrides)
        {
            EnableDpiAwareness();
            Settings settings = Settings.Load(overrides);
            if (IsSkippedByFocus(settings)) return 0;

            Color color;
            if (mode.Equals("done", StringComparison.OrdinalIgnoreCase)) color = settings.ColorDone;
            else if (mode.Equals("ask", StringComparison.OrdinalIgnoreCase) ||
                     mode.Equals("question", StringComparison.OrdinalIgnoreCase) ||
                     mode.Equals("input", StringComparison.OrdinalIgnoreCase))
            {
                color = settings.ColorAsk;
                // An explicit --alpha on the command line still wins.
                if (!overrides.ContainsKey("alpha")) settings.Alpha = settings.AlphaAsk;
            }
            else if (mode.Equals("perm", StringComparison.OrdinalIgnoreCase) ||
                     mode.Equals("permission", StringComparison.OrdinalIgnoreCase))
            {
                color = settings.ColorPerm;
                if (!overrides.ContainsKey("alpha")) settings.Alpha = settings.AlphaPerm;
            }
            else color = ParseColor(mode, settings.ColorDone);

            Run(color, settings);
            return 0;
        }

        private static readonly string[] KnownKeys = {
            "alpha", "alpha_ask", "alpha_perm", "fade_in_ms", "hold_ms", "fade_out_ms",
            "dismiss_fade_ms", "min_visible_ms", "vignette",
            "color_done", "color_ask", "color_perm", "skip_if_focused"
        };

        /// <summary>
        /// `flash set color_ask #08A9FF` - rewrites one line of config.ini, leaving the
        /// comments around it intact.
        /// </summary>
        [MethodImpl(MethodImplOptions.NoInlining)]
        private static int SetConfig(List<string> positional)
        {
            // positional[0] is "set" itself.
            string key = positional.Count > 1 ? positional[1].Trim().TrimStart('-') : "";
            string val = positional.Count > 2 ? positional[2].Trim() : "";

            int eq = key.IndexOf('=');          // also accept `flash set color_ask=#08A9FF`
            if (eq > 0 && val.Length == 0) { val = key.Substring(eq + 1); key = key.Substring(0, eq); }

            if (key.Length == 0 || val.Length == 0)
            {
                MessageBox.Show(
                    "Usage:  flash set <key> <value>\r\n\r\nKeys:\r\n  " + string.Join("\r\n  ", KnownKeys) +
                    "\r\n\r\nExamples:\r\n  flash set color_ask #08A9FF\r\n  flash set alpha 0.35" +
                    "\r\n  flash set hold_ms 700",
                    "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Information);
                return 1;
            }

            bool known = false;
            foreach (string k in KnownKeys)
                if (string.Equals(k, key, StringComparison.OrdinalIgnoreCase)) { key = k; known = true; break; }
            if (!known)
            {
                MessageBox.Show("Unknown setting: " + key + "\r\n\r\nKnown keys:\r\n  " +
                    string.Join("\r\n  ", KnownKeys), "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return 1;
            }

            // Reject values that would silently fall back to a default later.
            if (key.StartsWith("color_", StringComparison.Ordinal))
            {
                Color probe = ParseColor(val, Color.Empty);
                if (probe == Color.Empty)
                {
                    MessageBox.Show("Not a colour: " + val + "\r\n\r\nUse a name (green, blue, violet, ...) or #RRGGBB.",
                        "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                    return 1;
                }
            }
            else if (!key.Equals("skip_if_focused", StringComparison.Ordinal))
            {
                double n;
                if (!double.TryParse(val, NumberStyles.Float, CultureInfo.InvariantCulture, out n))
                {
                    MessageBox.Show("Not a number: " + val, "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                    return 1;
                }
            }

            try
            {
                string path = Settings.ConfigPath();
                Settings.Load(null);                       // make sure the file exists
                var lines = new List<string>(File.ReadAllLines(path));
                bool replaced = false;
                for (int i = 0; i < lines.Count; i++)
                {
                    string t = lines[i].TrimStart();
                    if (t.StartsWith("#", StringComparison.Ordinal) || t.StartsWith(";", StringComparison.Ordinal)) continue;
                    int e = t.IndexOf('=');
                    if (e <= 0) continue;
                    if (string.Equals(t.Substring(0, e).Trim(), key, StringComparison.OrdinalIgnoreCase))
                    {
                        lines[i] = key + "=" + val;
                        replaced = true;
                        break;
                    }
                }
                if (!replaced) lines.Add(key + "=" + val);
                File.WriteAllLines(path, lines.ToArray());
            }
            catch (Exception ex) { LogError(ex); return 1; }

            // Confirm in the colour that was just set, so you see the change immediately.
            Settings updated = Settings.Load(null);
            Color show = updated.ColorDone;
            if (key == "color_ask") { show = updated.ColorAsk; updated.Alpha = updated.AlphaAsk; }
            else if (key == "color_perm") { show = updated.ColorPerm; updated.Alpha = updated.AlphaPerm; }
            else if (key == "alpha_ask") updated.Alpha = updated.AlphaAsk;
            else if (key == "alpha_perm") updated.Alpha = updated.AlphaPerm;
            Run(show, updated);
            return 0;
        }

        private static void Confirm(bool enabled, Settings settings)
        {
            Run(enabled ? settings.ColorDone : ParseColor("red", Color.Red), settings);
        }

        private static void Run(Color color, Settings settings)
        {
            ClaimSingleInstance();
            using (var ctx = new FlashContext(color, settings))
            {
                Application.Run(ctx);
            }
        }

        /// <summary>
        /// Enumerating processes costs more than everything else this program does, so
        /// only pay for it when a cheap mutex says a previous flash is still on screen.
        /// </summary>
        private static void ClaimSingleInstance()
        {
            bool createdNew = false;
            try { _instanceLock = new Mutex(true, @"Local\ClaudeFlash.Overlay", out createdNew); }
            catch { return; }

            if (createdNew) return;

            KillOtherInstances();
            try { _instanceLock.WaitOne(500); }
            catch (AbandonedMutexException) { }
            catch { }
        }

        // ---- process helpers ---------------------------------------------------

        [MethodImpl(MethodImplOptions.NoInlining)]
        private static void Relaunch(string[] argv)
        {
            var passthrough = new List<string>();
            foreach (string a in argv)
            {
                if (a != null && a.Trim().TrimStart('-', '/').Equals("bg", StringComparison.OrdinalIgnoreCase)) continue;
                passthrough.Add(Quote(a));
            }

            try
            {
                string self = Assembly.GetEntryAssembly().Location;   // not Application.ExecutablePath: that would load WinForms
                var psi = new ProcessStartInfo(self, string.Join(" ", passthrough.ToArray()));
                // UseShellExecute is what makes this actually detached. With it false the
                // child inherits our stdio handles, and the caller - Claude Code's hook
                // runner - blocks reading them until the flash finishes, which is exactly
                // the delay --bg exists to avoid.
                psi.UseShellExecute = true;
                psi.WindowStyle = ProcessWindowStyle.Hidden;
                psi.WorkingDirectory = Path.GetDirectoryName(self);
                Process.Start(psi);
            }
            catch { /* a missed flash is never worth an error dialog */ }
        }

        private static string Quote(string s)
        {
            if (string.IsNullOrEmpty(s)) return "\"\"";
            return s.IndexOf(' ') >= 0 ? "\"" + s + "\"" : s;
        }

        // A second flash while one is still fading would stack two tints and show the
        // stale color, so the newest event always wins.
        private static void KillOtherInstances()
        {
            Process me = Process.GetCurrentProcess();
            string myPath;
            try { myPath = me.MainModule.FileName; }
            catch { return; }

            Process[] siblings;
            try { siblings = Process.GetProcessesByName(me.ProcessName); }
            catch { return; }

            foreach (Process p in siblings)
            {
                if (p.Id == me.Id) continue;
                try
                {
                    if (string.Equals(p.MainModule.FileName, myPath, StringComparison.OrdinalIgnoreCase))
                    {
                        p.Kill();
                        p.WaitForExit(400);
                    }
                }
                catch { /* different user, already gone, or access denied - skip it */ }
            }
        }

        private static bool IsSkippedByFocus(Settings settings)
        {
            if (string.IsNullOrEmpty(settings.SkipIfFocused)) return false;
            try
            {
                IntPtr hwnd = Native.GetForegroundWindow();
                if (hwnd == IntPtr.Zero) return false;
                uint pid;
                Native.GetWindowThreadProcessId(hwnd, out pid);
                string name = Process.GetProcessById((int)pid).ProcessName;
                foreach (string want in settings.SkipIfFocused.Split(','))
                {
                    string w = want.Trim();
                    if (w.Length == 0) continue;
                    if (w.EndsWith(".exe", StringComparison.OrdinalIgnoreCase)) w = w.Substring(0, w.Length - 4);
                    if (string.Equals(w, name, StringComparison.OrdinalIgnoreCase)) return true;
                }
            }
            catch { }
            return false;
        }

        // ---- enable / disable --------------------------------------------------

        internal static string DataDir()
        {
            string dir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "ClaudeFlash");
            try { Directory.CreateDirectory(dir); }
            catch { }
            return dir;
        }

        private static string DisabledMarker() { return Path.Combine(DataDir(), "disabled"); }

        internal static bool IsEnabled()
        {
            try { return !File.Exists(DisabledMarker()); }
            catch { return true; }
        }

        private static void SetEnabled(bool enabled)
        {
            try
            {
                if (enabled) File.Delete(DisabledMarker());
                else File.WriteAllText(DisabledMarker(),
                    "ClaudeFlash is off. Delete this file, or run: flash on" + Environment.NewLine);
            }
            catch { }
        }

        private static void ShowStatus(Settings settings)
        {
            string hooks = "not installed";
            try
            {
                string sp = Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), @".claude\settings.json");
                if (File.Exists(sp) && File.ReadAllText(sp).IndexOf("flash.exe", StringComparison.OrdinalIgnoreCase) >= 0)
                    hooks = "installed in " + sp;
            }
            catch { }

            var sb = new StringBuilder();
            sb.AppendLine(IsEnabled() ? "State:   ENABLED" : "State:   DISABLED  (run: flash on)");
            sb.AppendLine("Hooks:   " + hooks);
            sb.AppendLine("Exe:     " + Application.ExecutablePath);
            sb.AppendLine("Config:  " + Settings.ConfigPath());
            sb.AppendLine();
            sb.AppendLine("alpha=" + settings.Alpha.ToString("0.###", CultureInfo.InvariantCulture) +
                          "  vignette=" + settings.Vignette.ToString("0.###", CultureInfo.InvariantCulture));
            sb.AppendLine("fade_in=" + settings.FadeInMs + "ms  hold=" + settings.HoldMs +
                          "ms  fade_out=" + settings.FadeOutMs + "ms");
            sb.AppendLine("done=" + Hex(settings.ColorDone) + "  ask=" + Hex(settings.ColorAsk));

            MessageBox.Show(sb.ToString(), "ClaudeFlash", MessageBoxButtons.OK, MessageBoxIcon.Information);
        }

        private static string Hex(Color c)
        {
            return "#" + c.R.ToString("X2") + c.G.ToString("X2") + c.B.ToString("X2");
        }

        internal static Color ParseColor(string s, Color fallback)
        {
            if (string.IsNullOrEmpty(s)) return fallback;
            s = s.Trim();
            try
            {
                switch (s.ToLowerInvariant())
                {
                    case "green": return ColorTranslator.FromHtml("#00FF5A");
                    case "amber":
                    case "orange": return ColorTranslator.FromHtml("#FFAA00");
                    case "red": return ColorTranslator.FromHtml("#FF3B30");
                    case "blue": return ColorTranslator.FromHtml("#2E9BFF");
                    case "purple": return ColorTranslator.FromHtml("#B368FF");
                    case "violet": return ColorTranslator.FromHtml("#A855F7");
                    case "lavender": return ColorTranslator.FromHtml("#B9A7FF");
                    case "indigo": return ColorTranslator.FromHtml("#7C6BFF");
                    case "teal": return ColorTranslator.FromHtml("#00C9A7");
                    case "pink": return ColorTranslator.FromHtml("#FF7AB8");
                    case "cyan": return ColorTranslator.FromHtml("#00E5FF");
                    case "white": return ColorTranslator.FromHtml("#FFFFFF");
                }
                // Only add the # when the text really is six hex digits. Testing length
                // alone turned every six-letter colour name into "#violet", which failed
                // to parse and silently fell back to green.
                if (!s.StartsWith("#", StringComparison.Ordinal) && s.Length == 6 && IsHex(s)) s = "#" + s;
                return ColorTranslator.FromHtml(s);
            }
            catch { return fallback; }
        }

        private static bool IsHex(string s)
        {
            foreach (char c in s)
                if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'))) return false;
            return true;
        }

        private static void EnableDpiAwareness()
        {
            // Must happen before any window exists, or a scaled display would report
            // virtualized coordinates and the overlay would miss part of the screen.
            try { if (Native.SetProcessDpiAwarenessContext(Native.DpiPerMonitorAwareV2)) return; }
            catch { }
            try { Native.SetProcessDPIAware(); }
            catch { }
        }
    }

    // =========================================================================

    internal sealed class Settings
    {
        public double Alpha = 0.28;
        // Blue reads as blue only while it stays saturated, and a saturated tint at the
        // green's opacity is harsher than it needs to be. Lightening the colour instead
        // just turns the wash white, so the ask colour gets its own alpha.
        public double AlphaAsk = 0.20;
        public int FadeInMs = 70;
        public int HoldMs = 420;
        public int FadeOutMs = 560;
        public int DismissFadeMs = 110;
        public int MinVisibleMs = 120;
        public int MaxMs = 6000;
        public double Vignette = 0.32;
        public Color ColorDone = ColorTranslator.FromHtml("#00FF5A");
        public Color ColorAsk = ColorTranslator.FromHtml("#08A9FF");
        public Color ColorPerm = ColorTranslator.FromHtml("#A855F7");
        public double AlphaPerm = 0.17;
        public string SkipIfFocused = "";

        private const string Template =
            "# ClaudeFlash settings. Edit, save, done - the next flash picks it up.\r\n" +
            "\r\n" +
            "# Peak opacity of the tint, 0..1. Higher = harder to miss.\r\n" +
            "alpha=0.28\r\n" +
            "\r\n" +
            "# Opacity for the 'ask' colour. Blue needs to stay saturated to read as blue\r\n" +
            "# rather than white haze, so soften it here instead of lightening the colour.\r\n" +
            "alpha_ask=0.20\r\n" +
            "\r\n" +
            "# Animation, in milliseconds.\r\n" +
            "fade_in_ms=70\r\n" +
            "hold_ms=420\r\n" +
            "fade_out_ms=560\r\n" +
            "\r\n" +
            "# How fast it vanishes once you click or type.\r\n" +
            "dismiss_fade_ms=110\r\n" +
            "\r\n" +
            "# Ignore dismiss input for this long, so a keystroke already in flight\r\n" +
            "# cannot kill the flash before you see it.\r\n" +
            "min_visible_ms=120\r\n" +
            "\r\n" +
            "# 0 = flat tint. Higher = center stays clearer than the edges, which keeps\r\n" +
            "# whatever you are reading legible while the flash is up.\r\n" +
            "vignette=0.32\r\n" +
            "\r\n" +
            "# Colors.\r\n" +
            "color_done=#00FF5A\r\n" +
            "color_ask=#08A9FF\r\n" +
            "\r\n" +
            "# Shown when Claude asks permission to run something.\r\n" +
            "color_perm=#A855F7\r\n" +
            "alpha_perm=0.17\r\n" +
            "\r\n" +
            "# Comma-separated process names. If one of them owns the focused window when\r\n" +
            "# the flash fires, it is skipped - you were already looking at it.\r\n" +
            "# Example: skip_if_focused=WindowsTerminal,Claude\r\n" +
            "skip_if_focused=\r\n";

        internal static string ConfigPath()
        {
            return Path.Combine(Program.DataDir(), "config.ini");
        }

        internal static Settings Load(Dictionary<string, string> overrides)
        {
            var s = new Settings();
            var values = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

            try
            {
                string path = ConfigPath();
                if (!File.Exists(path)) File.WriteAllText(path, Template);

                foreach (string line in File.ReadAllLines(path))
                {
                    string t = line.Trim();
                    if (t.Length == 0 || t.StartsWith("#", StringComparison.Ordinal) ||
                        t.StartsWith(";", StringComparison.Ordinal)) continue;
                    int eq = t.IndexOf('=');
                    if (eq <= 0) continue;
                    values[t.Substring(0, eq).Trim()] = t.Substring(eq + 1).Trim();
                }
            }
            catch { /* fall back to the compiled-in defaults */ }

            if (overrides != null)
                foreach (KeyValuePair<string, string> kv in overrides) values[kv.Key] = kv.Value;

            s.Alpha = Clamp(GetDouble(values, "alpha", s.Alpha), 0.02, 1.0);
            s.AlphaAsk = Clamp(GetDouble(values, "alpha_ask", s.AlphaAsk), 0.02, 1.0);
            s.AlphaPerm = Clamp(GetDouble(values, "alpha_perm", s.AlphaPerm), 0.02, 1.0);
            s.Vignette = Clamp(GetDouble(values, "vignette", s.Vignette), 0.0, 0.9);
            s.FadeInMs = (int)Clamp(GetDouble(values, "fade_in_ms", s.FadeInMs), 0, 5000);
            s.HoldMs = (int)Clamp(GetDouble(values, "hold_ms", s.HoldMs), 0, 10000);
            s.FadeOutMs = (int)Clamp(GetDouble(values, "fade_out_ms", s.FadeOutMs), 1, 10000);
            s.DismissFadeMs = (int)Clamp(GetDouble(values, "dismiss_fade_ms", s.DismissFadeMs), 1, 2000);
            s.MinVisibleMs = (int)Clamp(GetDouble(values, "min_visible_ms", s.MinVisibleMs), 0, 2000);
            s.ColorDone = Program.ParseColor(GetString(values, "color_done", null), s.ColorDone);
            s.ColorAsk = Program.ParseColor(GetString(values, "color_ask", null), s.ColorAsk);
            s.ColorPerm = Program.ParseColor(GetString(values, "color_perm", null), s.ColorPerm);
            s.SkipIfFocused = GetString(values, "skip_if_focused", "");
            s.MaxMs = s.FadeInMs + s.HoldMs + s.FadeOutMs + 3000;

            return s;
        }

        private static double Clamp(double v, double lo, double hi)
        {
            return v < lo ? lo : (v > hi ? hi : v);
        }

        private static double GetDouble(Dictionary<string, string> v, string key, double fallback)
        {
            string raw;
            double parsed;
            if (v.TryGetValue(key, out raw) &&
                double.TryParse(raw, NumberStyles.Float, CultureInfo.InvariantCulture, out parsed)) return parsed;
            return fallback;
        }

        private static string GetString(Dictionary<string, string> v, string key, string fallback)
        {
            string raw;
            return v.TryGetValue(key, out raw) ? raw : fallback;
        }
    }

    // =========================================================================

    /// <summary>Drives one overlay per monitor through fade-in, hold, fade-out.</summary>
    internal sealed class FlashContext : ApplicationContext
    {
        private readonly Settings _s;
        private readonly List<Overlay> _overlays = new List<Overlay>();
        private readonly Stopwatch _clock = new Stopwatch();
        private readonly System.Windows.Forms.Timer _timer;

        private bool _dismissing;
        private double _dismissStartMs;
        private double _dismissStartAlpha;
        private bool _finished;

        private bool _inputPrimed;

        public FlashContext(Color color, Settings settings)
        {
            _s = settings;

            foreach (Screen screen in Screen.AllScreens)
            {
                Rectangle b = screen.Bounds;
                Bitmap layer = BuildLayer(b.Width, b.Height, color, _s.Vignette);
                var overlay = new Overlay(b, layer);
                overlay.SetAlpha(0);
                overlay.Show();
                _overlays.Add(overlay);
            }

            PrimeInput();

            _timer = new System.Windows.Forms.Timer();
            _timer.Interval = 15;
            _timer.Tick += OnTick;
            _clock.Start();
            _timer.Start();
            OnTick(null, EventArgs.Empty);
        }

        private void OnTick(object sender, EventArgs e)
        {
            double t = _clock.Elapsed.TotalMilliseconds;
            if (t > _s.MaxMs) { Finish(); return; }

            if (t >= _s.MinVisibleMs && AnyInputSinceLastCheck()) Dismiss();

            double a;
            if (_dismissing)
            {
                double d = (t - _dismissStartMs) / _s.DismissFadeMs;
                if (d >= 1.0) { Finish(); return; }
                a = _dismissStartAlpha * (1.0 - EaseOut(d));
            }
            else if (t < _s.FadeInMs)
            {
                a = _s.Alpha * EaseOut(t / _s.FadeInMs);
            }
            else if (t < _s.FadeInMs + _s.HoldMs)
            {
                a = _s.Alpha;
            }
            else
            {
                double d = (t - _s.FadeInMs - _s.HoldMs) / _s.FadeOutMs;
                if (d >= 1.0) { Finish(); return; }
                a = _s.Alpha * (1.0 - EaseInOut(d));
            }

            byte level = (byte)Math.Max(0, Math.Min(255, (int)(a * 255.0 + 0.5)));
            foreach (Overlay o in _overlays) o.SetAlpha(level);
        }

        private double CurrentAlpha()
        {
            double t = _clock.Elapsed.TotalMilliseconds;
            if (t < _s.FadeInMs) return _s.Alpha * EaseOut(t / Math.Max(1, _s.FadeInMs));
            if (t < _s.FadeInMs + _s.HoldMs) return _s.Alpha;
            double d = (t - _s.FadeInMs - _s.HoldMs) / _s.FadeOutMs;
            return _s.Alpha * (1.0 - EaseInOut(Math.Min(1.0, d)));
        }

        private void Dismiss()
        {
            if (_dismissing || _finished) return;
            if (_clock.Elapsed.TotalMilliseconds < _s.MinVisibleMs) return;
            _dismissStartAlpha = CurrentAlpha();
            _dismissStartMs = _clock.Elapsed.TotalMilliseconds;
            _dismissing = true;
        }

        private void Finish()
        {
            if (_finished) return;
            _finished = true;
            _timer.Stop();
            foreach (Overlay o in _overlays) o.Dispose();
            _overlays.Clear();
            ExitThread();
        }

        private static double EaseOut(double x) { return 1.0 - Math.Pow(1.0 - x, 3.0); }

        private static double EaseInOut(double x)
        {
            return x < 0.5 ? 4.0 * x * x * x : 1.0 - Math.Pow(-2.0 * x + 2.0, 3.0) / 2.0;
        }

        // ---- dismiss-on-any-input ---------------------------------------------
        //
        // The overlay is click-through, so input never reaches it and we have to look
        // for it some other way.
        //
        // This deliberately does NOT use SetWindowsHookEx(WH_KEYBOARD_LL). A global
        // keyboard hook is the defining behaviour of a keylogger, and antivirus treats
        // it that way - an unsigned binary that installs one is asking to be
        // quarantined on someone else's machine. Polling GetAsyncKeyState needs no
        // hook, installs nothing, and cannot see input outside the ~1 second the flash
        // is on screen.
        //
        // The low bit of GetAsyncKeyState means "pressed since the last call", so one
        // priming sweep at startup stops a key that was already held from dismissing
        // the flash before it is even visible. Nothing is recorded: the loop stops at
        // the first key that changed and never keeps which one it was.

        private void PrimeInput()
        {
            for (int vk = 1; vk < 256; vk++) Native.GetAsyncKeyState(vk);
            _inputPrimed = true;
        }

        private bool AnyInputSinceLastCheck()
        {
            if (!_inputPrimed) { PrimeInput(); return false; }
            for (int vk = 1; vk < 256; vk++)
                if ((Native.GetAsyncKeyState(vk) & 0x0001) != 0) return true;
            return false;
        }

        // ---- the tint ----------------------------------------------------------

        /// <summary>
        /// Builds the per-pixel alpha layer: uniform color, alpha dipping toward the
        /// center. Drawn small and scaled up - a 3.3M pixel loop would be visible lag.
        /// </summary>
        private static Bitmap BuildLayer(int width, int height, Color color, double vignette)
        {
            const int sw = 320, sh = 200;

            var small = new Bitmap(sw, sh, PixelFormat.Format32bppArgb);
            var fx = new double[sw];
            var fy = new double[sh];
            for (int x = 0; x < sw; x++) { double d = 2.0 * (x + 0.5) / sw - 1.0; fx[x] = 1.0 - d * d; }
            for (int y = 0; y < sh; y++) { double d = 2.0 * (y + 0.5) / sh - 1.0; fy[y] = 1.0 - d * d; }

            BitmapData data = small.LockBits(new Rectangle(0, 0, sw, sh),
                ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
            try
            {
                var row = new byte[sw * 4];
                for (int y = 0; y < sh; y++)
                {
                    int i = 0;
                    for (int x = 0; x < sw; x++)
                    {
                        double p = 1.0 - vignette * fx[x] * fy[y];
                        if (p < 0) p = 0;
                        row[i++] = color.B;
                        row[i++] = color.G;
                        row[i++] = color.R;
                        row[i++] = (byte)(p * 255.0 + 0.5);
                    }
                    Marshal.Copy(row, 0, (IntPtr)(data.Scan0.ToInt64() + (long)y * data.Stride), row.Length);
                }
            }
            finally { small.UnlockBits(data); }

            var big = new Bitmap(width, height, PixelFormat.Format32bppArgb);
            using (Graphics g = Graphics.FromImage(big))
            using (var attrs = new ImageAttributes())
            {
                g.CompositingMode = CompositingMode.SourceCopy;
                g.InterpolationMode = InterpolationMode.HighQualityBilinear;
                g.PixelOffsetMode = PixelOffsetMode.HighQuality;
                attrs.SetWrapMode(WrapMode.TileFlipXY);   // stops the scaler from bleeding transparency in at the edges
                g.DrawImage(small, new Rectangle(0, 0, width, height), 0, 0, sw, sh, GraphicsUnit.Pixel, attrs);
            }
            small.Dispose();
            return big;
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                if (_timer != null) _timer.Dispose();
                foreach (Overlay o in _overlays) o.Dispose();
                _overlays.Clear();
            }
            base.Dispose(disposing);
        }
    }

    // =========================================================================

    /// <summary>
    /// One click-through, never-focused layered window covering a single monitor.
    /// Contents come from UpdateLayeredWindow, so there is no WinForms painting at all.
    /// </summary>
    internal sealed class Overlay : Form
    {
        private readonly Rectangle _bounds;
        private IntPtr _screenDc, _memDc, _hBitmap, _oldBitmap;
        private bool _released;

        public Overlay(Rectangle bounds, Bitmap layer)
        {
            _bounds = bounds;

            FormBorderStyle = FormBorderStyle.None;
            ShowInTaskbar = false;
            StartPosition = FormStartPosition.Manual;
            AutoScaleMode = AutoScaleMode.None;
            ControlBox = false;
            MinimizeBox = false;
            MaximizeBox = false;
            Text = "ClaudeFlash";
            Bounds = bounds;
            TopMost = true;

            IntPtr handle = Handle;   // force creation so UpdateLayeredWindow has a target
            GC.KeepAlive(handle);

            _screenDc = Native.GetDC(IntPtr.Zero);
            _memDc = Native.CreateCompatibleDC(_screenDc);
            _hBitmap = layer.GetHbitmap(Color.FromArgb(0));   // preserves the alpha channel
            _oldBitmap = Native.SelectObject(_memDc, _hBitmap);
            layer.Dispose();
        }

        protected override bool ShowWithoutActivation { get { return true; } }

        protected override CreateParams CreateParams
        {
            get
            {
                CreateParams cp = base.CreateParams;
                cp.ExStyle |= Native.WS_EX_LAYERED      // per-pixel alpha
                            | Native.WS_EX_TRANSPARENT  // clicks fall through to whatever is underneath
                            | Native.WS_EX_NOACTIVATE   // never takes keyboard focus
                            | Native.WS_EX_TOOLWINDOW   // stays out of alt-tab
                            | Native.WS_EX_TOPMOST;
                return cp;
            }
        }

        public void SetAlpha(byte alpha)
        {
            if (_released) return;

            var blend = new Native.BLENDFUNCTION
            {
                BlendOp = Native.AC_SRC_OVER,
                BlendFlags = 0,
                SourceConstantAlpha = alpha,
                AlphaFormat = Native.AC_SRC_ALPHA
            };
            var dst = new Native.POINT(_bounds.Left, _bounds.Top);
            var src = new Native.POINT(0, 0);
            var size = new Native.SIZE(_bounds.Width, _bounds.Height);

            Native.UpdateLayeredWindow(Handle, _screenDc, ref dst, ref size,
                _memDc, ref src, 0, ref blend, Native.ULW_ALPHA);
        }

        private void Release()
        {
            if (_released) return;
            _released = true;
            if (_memDc != IntPtr.Zero)
            {
                Native.SelectObject(_memDc, _oldBitmap);
                Native.DeleteDC(_memDc);
                _memDc = IntPtr.Zero;
            }
            if (_hBitmap != IntPtr.Zero) { Native.DeleteObject(_hBitmap); _hBitmap = IntPtr.Zero; }
            if (_screenDc != IntPtr.Zero) { Native.ReleaseDC(IntPtr.Zero, _screenDc); _screenDc = IntPtr.Zero; }
        }

        protected override void Dispose(bool disposing)
        {
            Release();
            base.Dispose(disposing);
        }
    }

    // =========================================================================

    internal static class Native
    {
        public const int WS_EX_TOPMOST = 0x00000008;
        public const int WS_EX_TRANSPARENT = 0x00000020;
        public const int WS_EX_TOOLWINDOW = 0x00000080;
        public const int WS_EX_LAYERED = 0x00080000;
        public const int WS_EX_NOACTIVATE = 0x08000000;

        public const int ULW_ALPHA = 0x00000002;
        public const byte AC_SRC_OVER = 0x00;
        public const byte AC_SRC_ALPHA = 0x01;

        public static readonly IntPtr DpiPerMonitorAwareV2 = new IntPtr(-4);

        [StructLayout(LayoutKind.Sequential)]
        public struct POINT
        {
            public int X, Y;
            public POINT(int x, int y) { X = x; Y = y; }
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct SIZE
        {
            public int cx, cy;
            public SIZE(int cx, int cy) { this.cx = cx; this.cy = cy; }
        }

        [StructLayout(LayoutKind.Sequential, Pack = 1)]
        public struct BLENDFUNCTION
        {
            public byte BlendOp;
            public byte BlendFlags;
            public byte SourceConstantAlpha;
            public byte AlphaFormat;
        }

        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool UpdateLayeredWindow(IntPtr hwnd, IntPtr hdcDst, ref POINT pptDst, ref SIZE psize,
            IntPtr hdcSrc, ref POINT pptSrc, int crKey, ref BLENDFUNCTION pblend, int dwFlags);

        [DllImport("user32.dll")]
        public static extern IntPtr GetDC(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

        [DllImport("user32.dll")]
        public static extern IntPtr GetForegroundWindow();

        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

        // Reads whether a key or mouse button changed state. Unlike a hook this
        // receives nothing, intercepts nothing, and is what games use for input.
        [DllImport("user32.dll")]
        public static extern short GetAsyncKeyState(int vKey);

        [DllImport("user32.dll")]
        public static extern bool SetProcessDpiAwarenessContext(IntPtr value);

        [DllImport("user32.dll")]
        public static extern bool SetProcessDPIAware();
        [DllImport("gdi32.dll")]
        public static extern IntPtr CreateCompatibleDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        public static extern bool DeleteDC(IntPtr hdc);

        [DllImport("gdi32.dll")]
        public static extern IntPtr SelectObject(IntPtr hdc, IntPtr hObject);

        [DllImport("gdi32.dll")]
        public static extern bool DeleteObject(IntPtr hObject);
    }
}
