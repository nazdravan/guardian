import 'package:flutter/material.dart';

import '../services/settings_service.dart';
import '../theme.dart';
import '../utils/snackbar_helper.dart';
import '../widgets/room_selector.dart';
import '../widgets/ui.dart';

/// The rules of one room. Policy is per-room now, not one global list, so this
/// screen is always about the room named in the app bar.
class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  final SettingsService _service = SettingsService();

  List<Map<String, dynamic>> _rooms = [];
  List<Map<String, dynamic>> _apps = [];
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
      final apps = room == null
          ? <Map<String, dynamic>>[]
          : await _service.applications(room['id'] as String);
      if (!mounted) return;
      setState(() {
        _rooms = rooms;
        _room = room;
        _apps = apps;
      });
    } on ApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.message;
        _rooms = [];
        _apps = [];
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

  Future<void> _addApplication() async {
    final room = _room;
    if (room == null) return;
    final controller = TextEditingController();
    var list = (room['mode'] as String?) ?? 'blacklist';

    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => StatefulBuilder(
        builder: (context, setDialogState) => AlertDialog(
          title: const Text('Add a rule'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(
                controller: controller,
                autofocus: true,
                style: monoStyle(size: 15),
                decoration: const InputDecoration(
                  labelText: 'Process name',
                  hintText: 'steam.exe',
                ),
              ),
              const SizedBox(height: 16),
              SegmentedButton<String>(
                segments: const [
                  ButtonSegment(value: 'blacklist', label: Text('Blocked')),
                  ButtonSegment(value: 'whitelist', label: Text('Allowed')),
                ],
                selected: {list},
                onSelectionChanged: (s) => setDialogState(() => list = s.first),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Add'),
            ),
          ],
        ),
      ),
    );

    final name = controller.text.trim();
    controller.dispose();
    if (confirmed != true || name.isEmpty) return;
    await _run(
      () => _service.addApplication(room['id'] as String, name, list),
      'Rule added',
    );
  }

  @override
  Widget build(BuildContext context) {
    final room = _room;
    final mode = (room?['mode'] as String?) ?? 'blacklist';

    return Scaffold(
      appBar: AppBar(
        title: RoomSelector(
          rooms: _rooms,
          selected: room,
          onChanged: (r) async {
            await _service.setRoomId(r?['id'] as String?);
          },
        ),
      ),
      floatingActionButton: room == null
          ? null
          : FloatingActionButton.extended(
              onPressed: _addApplication,
              icon: const Icon(Icons.add),
              label: const Text('Rule'),
            ),
      body: RefreshIndicator(onRefresh: _load, child: _buildBody(room, mode)),
    );
  }

  Widget _buildBody(Map<String, dynamic>? room, String mode) {
    if (_loading && room == null) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return EmptyState(
        icon: Icons.cloud_off,
        title: 'Could not load the rules',
        detail: _error,
      );
    }
    if (room == null) {
      return const EmptyState(
        icon: Icons.meeting_room_outlined,
        title: 'No rooms yet',
        detail: 'Create one on the Room tab, then put a computer in it.',
      );
    }

    final protection = (room['protection_enabled'] as bool?) == true;
    final active = _apps.where((a) => a['list'] == mode).toList();
    final other = _apps.where((a) => a['list'] != mode).toList();

    return ListView(
      padding: const EdgeInsets.only(bottom: 96),
      children: [
        _PolicySummary(mode: mode, protection: protection),
        if (_apps.isEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(32, 48, 32, 0),
            child: Text(
              mode == 'whitelist'
                  ? 'No allowed programs yet. In allowlist mode that closes '
                        'everything but the system — add the programs this room '
                        'may run.'
                  : 'No blocked programs yet. Add one with the Rule button.',
              textAlign: TextAlign.center,
              style: const TextStyle(color: GuardianColors.muted, height: 1.45),
            ),
          ),
        ..._section(mode == 'whitelist' ? 'Allowed' : 'Blocked', active, room),
        if (other.isNotEmpty)
          ..._section('Inactive in this mode', other, room, inMode: false),
      ],
    );
  }

  List<Widget> _section(
    String title,
    List<Map<String, dynamic>> apps,
    Map<String, dynamic> room, {
    bool inMode = true,
  }) {
    if (apps.isEmpty) return const [];
    apps.sort((a, b) {
      final ae = a['enabled'] as bool? ?? true;
      final be = b['enabled'] as bool? ?? true;
      if (ae != be) return ae ? -1 : 1;
      return (a['name'] as String).compareTo(b['name'] as String);
    });
    return [
      SectionLabel(
        title,
        trailing: Text(
          '${apps.length}',
          style: monoStyle(size: 12, color: GuardianColors.dim),
        ),
      ),
      Card(
        child: Column(
          children: [
            for (var i = 0; i < apps.length; i++) ...[
              if (i > 0) const Divider(indent: 16, endIndent: 16),
              _RuleRow(
                app: apps[i],
                inMode: inMode,
                onToggle: (v) => _run(
                  () => _service.setApplicationEnabled(
                    room['id'] as String,
                    apps[i]['id'] as String,
                    v,
                  ),
                  v ? 'Rule switched on' : 'Rule switched off',
                ),
                onDelete: () => _run(
                  () => _service.deleteApplication(
                    room['id'] as String,
                    apps[i]['id'] as String,
                  ),
                  'Rule removed',
                ),
              ),
            ],
          ],
        ),
      ),
    ];
  }
}

/// What the room does right now, in one line: which way round the list works
/// and whether anything is enforced.
class _PolicySummary extends StatelessWidget {
  const _PolicySummary({required this.mode, required this.protection});

  final String mode;
  final bool protection;

  @override
  Widget build(BuildContext context) {
    final whitelist = mode == 'whitelist';
    return Card(
      margin: const EdgeInsets.fromLTRB(12, 12, 12, 0),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            Icon(
              protection ? Icons.shield : Icons.shield_outlined,
              color: protection
                  ? GuardianColors.guardian
                  : GuardianColors.amber,
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    protection
                        ? (whitelist ? 'Allowlist' : 'Blocklist')
                        : 'Protection is off',
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  const SizedBox(height: 2),
                  Text(
                    protection
                        ? (whitelist
                              ? 'Only the allowed programs may run.'
                              : 'The blocked programs are closed.')
                        : 'These rules are stored but nothing is enforced.',
                    style: const TextStyle(
                      color: GuardianColors.muted,
                      fontSize: 13,
                    ),
                  ),
                ],
              ),
            ),
            StateBadge(
              protection ? 'Enforcing' : 'Off',
              color: protection
                  ? GuardianColors.guardian
                  : GuardianColors.amber,
            ),
          ],
        ),
      ),
    );
  }
}

class _RuleRow extends StatelessWidget {
  const _RuleRow({
    required this.app,
    required this.inMode,
    required this.onToggle,
    required this.onDelete,
  });

  final Map<String, dynamic> app;

  /// False for a rule on the other list: it is kept, but the room's current
  /// mode does not consult it, so it is not enforced whatever its switch says.
  final bool inMode;
  final ValueChanged<bool> onToggle;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final enabled = app['enabled'] as bool? ?? true;
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 4, 4, 4),
      child: Row(
        children: [
          Switch(value: enabled, onChanged: onToggle),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  app['name'] as String,
                  style: monoStyle(
                    size: 14,
                    color: enabled ? GuardianColors.ink : GuardianColors.dim,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
                Text(
                  !enabled
                      ? 'Switched off'
                      : inMode
                      ? 'Enforced'
                      : app['list'] == 'whitelist'
                      ? 'Applies in allowlist mode'
                      : 'Applies in blocklist mode',
                  style: const TextStyle(
                    fontSize: 12,
                    color: GuardianColors.muted,
                  ),
                ),
              ],
            ),
          ),
          IconButton(
            icon: const Icon(Icons.delete_outline, size: 20),
            color: GuardianColors.muted,
            tooltip: 'Remove rule',
            onPressed: onDelete,
          ),
        ],
      ),
    );
  }
}
