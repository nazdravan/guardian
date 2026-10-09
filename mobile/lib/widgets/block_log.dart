import 'package:flutter/material.dart';

import '../services/settings_service.dart';
import '../theme.dart';
import '../utils/format.dart';
import 'ui.dart';

/// Loads one page of the log for whatever the caller is showing — a room or
/// one computer. The two endpoints are separate, so the caller supplies it.
typedef ProcessEventLoader =
    Future<ProcessEventPage> Function(
      ProcessEventFilter filter,
      String? cursor,
    );

/// The ranges worth one tap. The default is the last day, because the question
/// this screen answers is almost always "what happened today".
enum LogPeriod {
  day('24 h', Duration(hours: 24)),
  week('7 days', Duration(days: 7)),
  month('30 days', Duration(days: 30)),
  all('All', null);

  const LogPeriod(this.label, this.span);
  final String label;
  final Duration? span;
}

/// The blocking log: what a policy actually closed. Only kills are here — an
/// allowed program, a system process or anything on the whitelist never is.
/// The agent folds repeated kills of one program into a single row with a
/// count, and ships them with its next sync, so a row can be ~20 s late.
///
/// The phone's version of the cabinet's `screens/blocklog.js`, used twice: as
/// the Log tab (one room) and from a computer's menu (one machine).
class BlockLog extends StatefulWidget {
  const BlockLog({
    super.key,
    required this.load,
    this.computers,
    this.header,
    this.now,
  });

  final ProcessEventLoader load;

  /// The room's machines, for the computer filter. Null on a computer's own
  /// log, which also hides the computer name on every row.
  final List<Map<String, dynamic>>? computers;

  /// Shown above the filters, scrolling with the list.
  final Widget? header;

  /// Fixes "now" for relative times. Tests only.
  final DateTime? now;

  @override
  State<BlockLog> createState() => _BlockLogState();
}

class _BlockLogState extends State<BlockLog> {
  final _process = TextEditingController();

  LogPeriod _period = LogPeriod.day;
  String? _reason;
  String? _computerId;

  List<Map<String, dynamic>> _rows = [];
  String? _cursor;
  bool _loading = true;
  bool _loadingMore = false;
  String? _error;

  /// Bumped on every fresh load, so an answer to an older filter that arrives
  /// late cannot overwrite the rows of the current one.
  int _generation = 0;

  bool get _byRoom => widget.computers != null;

  @override
  void initState() {
    super.initState();
    _reload();
  }

  @override
  void dispose() {
    _process.dispose();
    super.dispose();
  }

  ProcessEventFilter get _filter {
    final span = _period.span;
    return ProcessEventFilter(
      process: _process.text,
      reason: _reason,
      computerId: _computerId,
      since: span == null
          ? null
          : (widget.now ?? DateTime.now()).toUtc().subtract(span),
    );
  }

  Future<void> _reload() async {
    final generation = ++_generation;
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final page = await widget.load(_filter, null);
      if (!mounted || generation != _generation) return;
      setState(() {
        _rows = page.events;
        _cursor = page.nextCursor;
      });
    } on ApiException catch (e) {
      if (!mounted || generation != _generation) return;
      setState(() {
        _error = e.message;
        _rows = [];
        _cursor = null;
      });
    } finally {
      if (mounted && generation == _generation) {
        setState(() => _loading = false);
      }
    }
  }

  Future<void> _more() async {
    final cursor = _cursor;
    if (cursor == null || _loadingMore) return;
    final generation = _generation;
    setState(() => _loadingMore = true);
    try {
      final page = await widget.load(_filter, cursor);
      if (!mounted || generation != _generation) return;
      setState(() {
        _rows = [..._rows, ...page.events];
        _cursor = page.nextCursor;
      });
    } on ApiException catch (e) {
      if (!mounted) return;
      setState(() => _error = e.message);
    } finally {
      if (mounted) setState(() => _loadingMore = false);
    }
  }

  void _set(VoidCallback change) {
    setState(change);
    _reload();
  }

  @override
  Widget build(BuildContext context) {
    final now = widget.now ?? DateTime.now();
    final children = <Widget>[
      ?widget.header,
      _filters(),
      if (_error != null) ErrorBanner(_error!, onRetry: _reload),
    ];

    if (_loading && _rows.isEmpty) {
      children.add(
        const Padding(
          padding: EdgeInsets.all(48),
          child: Center(child: CircularProgressIndicator()),
        ),
      );
    } else if (_rows.isEmpty && _error == null) {
      children.add(_empty());
    } else {
      children.add(const SizedBox(height: 4));
      for (final row in _rows) {
        children.add(_LogRow(row: row, now: now, showComputer: _byRoom));
      }
      children.add(_footer());
    }

    return RefreshIndicator(
      onRefresh: _reload,
      child: ListView(
        physics: const AlwaysScrollableScrollPhysics(),
        padding: const EdgeInsets.only(bottom: 24),
        children: children,
      ),
    );
  }

  Widget _filters() {
    final computers = widget.computers ?? const [];
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _process,
            style: monoStyle(size: 14),
            textInputAction: TextInputAction.search,
            decoration: InputDecoration(
              isDense: true,
              hintText: 'Program, e.g. steam.exe',
              prefixIcon: const Icon(Icons.search, size: 20),
              suffixIcon: _process.text.isEmpty
                  ? null
                  : IconButton(
                      icon: const Icon(Icons.close, size: 18),
                      tooltip: 'Clear',
                      onPressed: () => _set(_process.clear),
                    ),
            ),
            onChanged: (_) => setState(() {}),
            onSubmitted: (_) => _reload(),
          ),
          const SizedBox(height: 10),
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              children: [
                for (final p in LogPeriod.values)
                  Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: ChoiceChip(
                      label: Text(p.label),
                      selected: _period == p,
                      showCheckmark: false,
                      onSelected: (_) => _set(() => _period = p),
                    ),
                  ),
                const SizedBox(width: 6),
                _MenuChip<String?>(
                  label: _reason == null ? 'Any reason' : reasonLabel(_reason),
                  selected: _reason,
                  options: [
                    (null, 'Any reason'),
                    for (final e in reasonLabels.entries) (e.key, e.value),
                  ],
                  onSelected: (v) => _set(() => _reason = v),
                ),
                if (_byRoom && computers.isNotEmpty) ...[
                  const SizedBox(width: 6),
                  _MenuChip<String?>(
                    label: _computerId == null
                        ? 'Every computer'
                        : computerName(
                            computers.firstWhere(
                              (c) => c['id'] == _computerId,
                              orElse: () => const {},
                            ),
                          ),
                    selected: _computerId,
                    options: [
                      (null, 'Every computer'),
                      for (final c in computers)
                        (c['id'] as String, computerName(c)),
                    ],
                    onSelected: (v) => _set(() => _computerId = v),
                  ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _empty() {
    return const Padding(
      padding: EdgeInsets.fromLTRB(32, 64, 32, 32),
      child: Column(
        children: [
          Icon(
            Icons.verified_user_outlined,
            size: 40,
            color: GuardianColors.dim,
          ),
          SizedBox(height: 16),
          Text(
            'Nothing was closed in this period',
            textAlign: TextAlign.center,
            style: TextStyle(fontWeight: FontWeight.w500, fontSize: 16),
          ),
          SizedBox(height: 8),
          Text(
            'Either nothing tried to run against the rules, or the machines '
            'have not reported since. An agent sends what it closed with its '
            'next sync.',
            textAlign: TextAlign.center,
            style: TextStyle(color: GuardianColors.muted, height: 1.45),
          ),
        ],
      ),
    );
  }

  Widget _footer() {
    if (_cursor != null) {
      return Padding(
        padding: const EdgeInsets.all(16),
        child: Center(
          child: OutlinedButton(
            onPressed: _loadingMore ? null : _more,
            child: _loadingMore
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Text('Load more'),
          ),
        ),
      );
    }
    return const Padding(
      padding: EdgeInsets.all(20),
      child: Text(
        'That is everything in this period. The log keeps 30 days.',
        textAlign: TextAlign.center,
        style: TextStyle(color: GuardianColors.dim, fontSize: 12),
      ),
    );
  }
}

/// One aggregated row: which program, on which machine, why, how many times,
/// and when. The relative time is the server's receipt time (`created_at`) —
/// the only clock that can be trusted; the agent's own span is shown beside it.
class _LogRow extends StatelessWidget {
  const _LogRow({
    required this.row,
    required this.now,
    required this.showComputer,
  });

  final Map<String, dynamic> row;
  final DateTime now;
  final bool showComputer;

  @override
  Widget build(BuildContext context) {
    final reason = row['reason'] as String?;
    final count = (row['count'] as num?)?.toInt() ?? 1;
    final computer = (row['computer_name'] as String?) ?? '';

    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      decoration: BoxDecoration(
        color: GuardianColors.panel,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: GuardianColors.line),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  (row['process'] as String?) ?? '?',
                  style: monoStyle(size: 15, weight: FontWeight.w500),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '×$count',
                style: monoStyle(size: 14, color: GuardianColors.muted),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              StateBadge(
                reasonLabel(reason),
                color: reason == 'locked'
                    ? GuardianColors.amber
                    : reason == 'overflow'
                    ? GuardianColors.danger
                    : null,
              ),
              const SizedBox(width: 10),
              if (showComputer && computer.isNotEmpty)
                Expanded(
                  child: Text(
                    computer,
                    style: const TextStyle(
                      color: GuardianColors.muted,
                      fontSize: 13,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                )
              else
                const Spacer(),
              Text(
                relativeTime(row['created_at'] as String?, now: now),
                style: monoStyle(size: 12, color: GuardianColors.muted),
              ),
            ],
          ),
          const SizedBox(height: 4),
          Text(
            '${formatDate(row['first_at'] as String?)}  '
            '${formatSpan(row['first_at'] as String?, row['last_at'] as String?)}',
            style: monoStyle(size: 11, color: GuardianColors.dim),
          ),
        ],
      ),
    );
  }
}

/// A chip that opens a short list of choices. The filter row on a phone is
/// too narrow for drop-downs side by side.
class _MenuChip<T> extends StatelessWidget {
  const _MenuChip({
    required this.label,
    required this.selected,
    required this.options,
    required this.onSelected,
  });

  final String label;

  /// The current value; null is the "everything" option, shown unhighlighted.
  final T? selected;
  final List<(T, String)> options;
  final ValueChanged<T> onSelected;

  @override
  Widget build(BuildContext context) {
    return InputChip(
      label: Text(label),
      selected: selected != null,
      showCheckmark: false,
      avatar: const Icon(Icons.expand_more, size: 18),
      onPressed: () async {
        final picked = await showModalBottomSheet<(T,)>(
          context: context,
          builder: (context) => SafeArea(
            child: ListView(
              shrinkWrap: true,
              children: [
                for (final (value, text) in options)
                  ListTile(
                    title: Text(text),
                    trailing: value == selected
                        ? const Icon(
                            Icons.check,
                            color: GuardianColors.guardian,
                          )
                        : null,
                    onTap: () => Navigator.pop(context, (value,)),
                  ),
              ],
            ),
          ),
        );
        if (picked != null) onSelected(picked.$1);
      },
    );
  }
}
