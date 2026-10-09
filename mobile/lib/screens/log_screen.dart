import 'package:flutter/material.dart';

import '../services/settings_service.dart';
import '../widgets/block_log.dart';
import '../widgets/room_selector.dart';
import '../widgets/ui.dart';

/// **Log** — what the selected room's policy closed: programs that tried to run
/// and were killed. The room's twin of the cabinet's Log tab.
class LogScreen extends StatefulWidget {
  const LogScreen({super.key});

  @override
  State<LogScreen> createState() => _LogScreenState();
}

class _LogScreenState extends State<LogScreen> {
  final SettingsService _service = SettingsService();

  List<Map<String, dynamic>> _rooms = [];
  List<Map<String, dynamic>> _computers = [];
  Map<String, dynamic>? _room;
  bool _loading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _service.addListener(_onServiceChanged);
    _load();
  }

  @override
  void dispose() {
    _service.removeListener(_onServiceChanged);
    super.dispose();
  }

  void _onServiceChanged() => _load();

  Future<void> _load() async {
    if (!mounted) return;
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final rooms = await _service.rooms();
      final room = pickRoom(rooms, _service.roomId);
      // The computer filter lists only this room's machines. A failure here
      // costs the filter, not the log, so it is not an error.
      var computers = <Map<String, dynamic>>[];
      try {
        computers = (await _service.computers())
            .where((c) => c['room_id'] == room?['id'])
            .toList();
      } on ApiException {
        computers = [];
      }
      if (!mounted) return;
      setState(() {
        _rooms = rooms;
        _room = room;
        _computers = computers;
      });
    } on ApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.message;
        _rooms = [];
        _room = null;
      });
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final room = _room;
    return Scaffold(
      appBar: AppBar(
        title: RoomSelector(
          rooms: _rooms,
          selected: room,
          onChanged: (r) => _service.setRoomId(r?['id'] as String?),
        ),
      ),
      body: _body(room),
    );
  }

  Widget _body(Map<String, dynamic>? room) {
    if (_loading && room == null) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return RefreshIndicator(
        onRefresh: _load,
        child: EmptyState(
          icon: Icons.cloud_off,
          title: 'Could not load the log',
          detail: _error,
        ),
      );
    }
    if (room == null) {
      return RefreshIndicator(
        onRefresh: _load,
        child: const EmptyState(
          icon: Icons.meeting_room_outlined,
          title: 'No rooms yet',
          detail:
              'The log is kept per room. Create one on the Room tab and put a '
              'computer in it.',
        ),
      );
    }
    final roomId = room['id'] as String;
    return BlockLog(
      // A new room is a new log: filters and pages do not carry over.
      key: ValueKey(roomId),
      computers: _computers,
      load: (filter, cursor) =>
          _service.roomProcessEvents(roomId, filter: filter, cursor: cursor),
    );
  }
}

/// One machine's log, opened from its card on the Computers tab. It finds the
/// machine's rows even when it is in no room.
class ComputerLogScreen extends StatelessWidget {
  const ComputerLogScreen({
    super.key,
    required this.computerId,
    required this.name,
  });

  final String computerId;
  final String name;

  @override
  Widget build(BuildContext context) {
    final service = SettingsService();
    return Scaffold(
      appBar: AppBar(title: Text(name)),
      body: BlockLog(
        load: (filter, cursor) => service.computerProcessEvents(
          computerId,
          filter: filter,
          cursor: cursor,
        ),
      ),
    );
  }
}
