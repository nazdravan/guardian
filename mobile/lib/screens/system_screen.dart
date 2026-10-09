import 'package:flutter/material.dart';

import '../services/settings_service.dart';
import '../theme.dart';
import '../utils/snackbar_helper.dart';
import '../widgets/room_selector.dart';
import '../widgets/ui.dart';

/// The room itself: whether it enforces anything, which way round its list
/// works, and whether the machines in it may be powered off. These were three
/// global settings on one server; each room carries its own now.
class SystemScreen extends StatefulWidget {
  const SystemScreen({super.key});

  @override
  State<SystemScreen> createState() => _SystemScreenState();
}

class _SystemScreenState extends State<SystemScreen> {
  final SettingsService _service = SettingsService();

  List<Map<String, dynamic>> _rooms = [];
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
      if (!mounted) return;
      setState(() {
        _rooms = rooms;
        _room = pickRoom(rooms, _service.roomId);
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

  Future<void> _run(Future<void> Function() action, String success) async {
    try {
      await action();
      if (!mounted) return;
      showSnackBarMessage(context, message: success);
      await _load();
    } on ApiException catch (e) {
      if (!mounted) return;
      showSnackBarMessage(context, message: e.message, kind: MessageKind.error);
    }
  }

  Future<String?> _askForName(String title, String initial) async {
    final controller = TextEditingController(text: initial);
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: const InputDecoration(
            labelText: 'Room name',
            hintText: 'Kids room',
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Save'),
          ),
        ],
      ),
    );
    final name = controller.text.trim();
    controller.dispose();
    return ok == true && name.isNotEmpty ? name : null;
  }

  Future<void> _createRoom() async {
    final name = await _askForName('New room', '');
    if (name == null) return;
    await _run(() async {
      final room = await _service.createRoom(name);
      await _service.setRoomId(room['id'] as String?);
    }, 'Room created');
  }

  Future<void> _deleteRoom(Map<String, dynamic> room) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Delete ${room['name']}?'),
        content: const Text(
          'Its rules are deleted with it. Computers in the room are not: they stay in the '
          'account, unassigned, and stop enforcing anything until they are put somewhere.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: FilledButton.styleFrom(
              backgroundColor: GuardianColors.danger,
            ),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await _run(() async {
      await _service.deleteRoom(room['id'] as String);
      await _service.setRoomId(null);
    }, 'Room deleted');
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: RoomSelector(
          rooms: _rooms,
          selected: _room,
          onChanged: (r) async => _service.setRoomId(r?['id'] as String?),
        ),
        actions: [
          IconButton(
            icon: const Icon(Icons.add),
            tooltip: 'New room',
            onPressed: _createRoom,
          ),
        ],
      ),
      body: RefreshIndicator(onRefresh: _load, child: _buildBody()),
    );
  }

  Widget _buildBody() {
    if (_loading && _room == null) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return EmptyState(
        icon: Icons.cloud_off,
        title: 'Could not load the room',
        detail: _error,
      );
    }
    final room = _room;
    if (room == null) {
      return EmptyState(
        icon: Icons.meeting_room_outlined,
        title: 'No rooms yet',
        detail:
            'A room holds one set of rules. Create one, then put computers in it.',
        action: FilledButton.icon(
          onPressed: _createRoom,
          icon: const Icon(Icons.add),
          label: const Text('New room'),
        ),
      );
    }

    final id = room['id'] as String;
    final mode = (room['mode'] as String?) ?? 'blacklist';
    final protection = room['protection_enabled'] as bool? ?? false;
    final power = room['power_allowed'] as bool? ?? true;

    return ListView(
      padding: const EdgeInsets.only(top: 6, bottom: 24),
      children: [
        const SectionLabel('Enforcement'),
        Card(
          child: SwitchListTile(
            title: const Text('Protection'),
            subtitle: Text(
              protection
                  ? 'The rules are being enforced'
                  : 'Nothing is enforced in this room',
            ),
            secondary: Icon(
              protection ? Icons.shield : Icons.shield_outlined,
              color: protection
                  ? GuardianColors.guardian
                  : GuardianColors.amber,
            ),
            value: protection,
            onChanged: (v) => _run(
              () => _service
                  .updateRoom(id, {'protection_enabled': v})
                  .then((_) {}),
              v ? 'Protection on' : 'Protection off',
            ),
          ),
        ),
        Card(
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text('Mode', style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 4),
                Text(
                  mode == 'whitelist'
                      ? 'Only the listed programs may run. Everything else is closed.'
                      : 'The listed programs are closed. Everything else may run.',
                  style: const TextStyle(
                    color: GuardianColors.muted,
                    fontSize: 13,
                  ),
                ),
                const SizedBox(height: 14),
                SegmentedButton<String>(
                  segments: const [
                    ButtonSegment(value: 'blacklist', label: Text('Blocklist')),
                    ButtonSegment(value: 'whitelist', label: Text('Allowlist')),
                  ],
                  selected: {mode},
                  onSelectionChanged: (s) => _run(
                    () =>
                        _service.updateRoom(id, {'mode': s.first}).then((_) {}),
                    'Mode changed',
                  ),
                ),
              ],
            ),
          ),
        ),
        Card(
          child: SwitchListTile(
            title: const Text('Power allowed'),
            subtitle: Text(
              power
                  ? 'Machines in this room may stay on'
                  : 'Machines in this room shut themselves down',
            ),
            secondary: Icon(
              Icons.power_settings_new,
              color: power ? GuardianColors.muted : GuardianColors.danger,
            ),
            value: power,
            onChanged: (v) => _run(
              () => _service.updateRoom(id, {'power_allowed': v}).then((_) {}),
              v ? 'Power allowed' : 'Shutdown requested',
            ),
          ),
        ),
        const SectionLabel('Room'),
        Card(
          child: Column(
            children: [
              ListTile(
                leading: const Icon(Icons.edit_outlined),
                title: const Text('Rename room'),
                onTap: () async {
                  final name = await _askForName(
                    'Rename room',
                    room['name'] as String,
                  );
                  if (name == null) return;
                  await _run(
                    () => _service.updateRoom(id, {'name': name}).then((_) {}),
                    'Room renamed',
                  );
                },
              ),
              const Divider(indent: 16, endIndent: 16),
              ListTile(
                leading: const Icon(
                  Icons.delete_outline,
                  color: GuardianColors.danger,
                ),
                title: const Text(
                  'Delete room',
                  style: TextStyle(color: GuardianColors.danger),
                ),
                onTap: () => _deleteRoom(room),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
