/// Words and times as a person reads them. Kept in step with the cabinet's
/// `server/webui/format.js`, so the phone and the browser say the same thing
/// about the same row.
library;

/// Why a process was closed, in the words somebody who did not design the
/// schema would use. "whitelist" as a reason reads like the opposite of what
/// happened: the program was killed for being absent from it.
const Map<String, String> reasonLabels = {
  'blacklist': 'Blocked by the list',
  'whitelist': 'Not on the list',
  'locked': 'Computer locked',
  'overflow': 'Dropped by the agent',
};

String reasonLabel(String? reason) =>
    reasonLabels[reason] ?? (reason ?? 'Unknown');

/// A machine's name as the account sees it: the name given in the cabinet,
/// else the hostname the agent reported.
String computerName(Map<String, dynamic> computer) {
  final display = (computer['display_name'] as String?) ?? '';
  if (display.isNotEmpty) return display;
  final hostname = (computer['hostname'] as String?) ?? '';
  return hostname.isNotEmpty ? hostname : 'Unnamed computer';
}

/// "just now", "5 min ago", "3 h ago", "2 d ago", then a date.
String relativeTime(String? iso, {DateTime? now}) {
  if (iso == null) return 'never';
  final then = DateTime.tryParse(iso);
  if (then == null) return 'never';
  final seconds = (now ?? DateTime.now()).toUtc().difference(then.toUtc());
  if (seconds.inSeconds < 45) return 'just now';
  final minutes = (seconds.inSeconds / 60).round();
  if (minutes < 60) return '$minutes min ago';
  final hours = (minutes / 60).round();
  if (hours < 24) return '$hours h ago';
  final days = (hours / 24).round();
  if (days < 30) return '$days d ago';
  return formatDate(iso);
}

const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

/// `9 Oct 2026`, in the phone's own time zone.
String formatDate(String? iso) {
  final t = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (t == null) return '—';
  return '${t.day} ${_months[t.month - 1]} ${t.year}';
}

/// `14:05`, in the phone's own time zone.
String formatTime(String? iso) {
  final t = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  if (t == null) return '—';
  String two(int v) => v.toString().padLeft(2, '0');
  return '${two(t.hour)}:${two(t.minute)}';
}

/// The span an aggregated row covers: `14:00`, or `14:00–14:46` when the
/// kills ran over more than a minute. These come from the agent's clock, so
/// they are shown, never sorted on.
String formatSpan(String? firstAt, String? lastAt) {
  final first = formatTime(firstAt);
  final last = formatTime(lastAt);
  if (last == '—' || last == first) return first;
  return '$first–$last';
}
