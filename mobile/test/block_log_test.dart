import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:procsentinel_mobile/services/settings_service.dart';
import 'package:procsentinel_mobile/theme.dart';
import 'package:procsentinel_mobile/widgets/block_log.dart';

Map<String, dynamic> _row(
  String id,
  String process, {
  String reason = 'whitelist',
}) => {
  'id': id,
  'computer_id': 'c1',
  'computer_name': 'LAB-A-01',
  'room_id': 'room-1',
  'process': process,
  'reason': reason,
  'count': 3,
  'first_at': '2026-10-09T14:00:00Z',
  'last_at': '2026-10-09T14:02:00Z',
  'created_at': '2026-10-09T14:02:30Z',
};

/// A loader that records what it was asked and answers from a script.
class _Loader {
  _Loader(this.pages);

  final List<ProcessEventPage> pages;
  final List<(ProcessEventFilter, String?)> calls = [];

  Future<ProcessEventPage> call(
    ProcessEventFilter filter,
    String? cursor,
  ) async {
    calls.add((filter, cursor));
    if (pages.isEmpty) return const ProcessEventPage(events: []);
    return pages.removeAt(0);
  }
}

Widget _app(Widget child) => MaterialApp(
  theme: buildGuardianTheme(),
  home: Scaffold(body: child),
);

final _now = DateTime.utc(2026, 10, 9, 14, 7);

void main() {
  testWidgets('rows show the program, why, how often and where', (
    tester,
  ) async {
    final loader = _Loader([
      ProcessEventPage(events: [_row('e1', 'steam.exe')]),
    ]);
    await tester.pumpWidget(
      _app(BlockLog(load: loader.call, computers: const [], now: _now)),
    );
    await tester.pumpAndSettle();

    expect(find.text('steam.exe'), findsOneWidget);
    // "whitelist" as a reason means the program was absent from the list.
    expect(find.text('Not on the list'), findsOneWidget);
    expect(find.text('×3'), findsOneWidget);
    expect(find.text('LAB-A-01'), findsOneWidget);
    expect(find.text('5 min ago'), findsOneWidget);
    // The default period is the last day.
    final since = loader.calls.single.$1.since!;
    expect(_now.difference(since), const Duration(hours: 24));
  });

  testWidgets('a computer\'s own log does not repeat its name on every row', (
    tester,
  ) async {
    final loader = _Loader([
      ProcessEventPage(events: [_row('e1', 'steam.exe')]),
    ]);
    await tester.pumpWidget(_app(BlockLog(load: loader.call, now: _now)));
    await tester.pumpAndSettle();

    expect(find.text('LAB-A-01'), findsNothing);
    expect(find.text('Every computer'), findsNothing);
  });

  testWidgets('load more follows the cursor and appends', (tester) async {
    final loader = _Loader([
      ProcessEventPage(events: [_row('e1', 'steam.exe')], nextCursor: 'cur-2'),
      ProcessEventPage(events: [_row('e2', 'roblox.exe')]),
    ]);
    await tester.pumpWidget(
      _app(BlockLog(load: loader.call, computers: const [], now: _now)),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Load more'));
    await tester.pumpAndSettle();

    expect(loader.calls.last.$2, 'cur-2');
    expect(find.text('steam.exe'), findsOneWidget);
    expect(find.text('roblox.exe'), findsOneWidget);
    expect(find.text('Load more'), findsNothing);
    expect(find.textContaining('The log keeps 30 days'), findsOneWidget);
  });

  testWidgets('choosing a period reloads from the first page', (tester) async {
    final loader = _Loader([
      ProcessEventPage(events: [_row('e1', 'steam.exe')]),
      const ProcessEventPage(events: []),
    ]);
    await tester.pumpWidget(
      _app(BlockLog(load: loader.call, computers: const [], now: _now)),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('All'));
    await tester.pumpAndSettle();

    expect(loader.calls, hasLength(2));
    expect(loader.calls.last.$1.since, isNull);
    expect(loader.calls.last.$2, isNull);
    expect(find.text('Nothing was closed in this period'), findsOneWidget);
  });

  testWidgets('a failed load says what the server said', (tester) async {
    Future<ProcessEventPage> failing(ProcessEventFilter f, String? c) =>
        Future.error(ApiException('The server did not answer in time.'));
    await tester.pumpWidget(
      _app(BlockLog(load: failing, computers: const [], now: _now)),
    );
    await tester.pumpAndSettle();

    expect(find.text('The server did not answer in time.'), findsOneWidget);
    // An outage is not "nothing was closed".
    expect(find.text('Nothing was closed in this period'), findsNothing);
  });
}
