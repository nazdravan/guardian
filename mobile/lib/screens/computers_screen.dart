import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../services/settings_service.dart';
import '../theme.dart';
import '../utils/format.dart';
import '../utils/snackbar_helper.dart';
import '../widgets/ui.dart';
import 'log_screen.dart';

/// The account's computers. A machine belongs to the account, not to a room —
/// moving it between rooms reinstalls nothing, it just receives a different
/// policy on its next sync.
class ComputersScreen extends StatefulWidget {
  const ComputersScreen({super.key});

  @override
  State<ComputersScreen> createState() => _ComputersScreenState();
}

class _ComputersScreenState extends State<ComputersScreen> {
  final SettingsService _service = SettingsService();

  List<Map<String, dynamic>> _computers = [];
  List<Map<String, dynamic>> _rooms = [];
  bool _loading = true;
  String? _error;
  Timer? _refresh;

  @override
  void initState() {
    super.initState();
    _service.addListener(_onServiceChanged);
    _load();
    _refresh = Timer.periodic(const Duration(seconds: 15), (_) => _load());
  }

  @override
  void dispose() {
    _service.removeListener(_onServiceChanged);
    _refresh?.cancel();
    super.dispose();
  }

  void _onServiceChanged() => _load();

  Future<void> _load() async {
    if (!mounted) return;
    try {
      final computers = await _service.computers();
      final rooms = await _service.rooms();
      if (!mounted) return;
      setState(() {
        _computers = computers;
        _rooms = rooms;
        _error = null;
      });
    } on ApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.message;
        _computers = [];
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

  /// A machine is "online" if it has synced within three poll intervals. The
  /// server records last_seen_at at most every 30 seconds, so anything much
  /// tighter than a minute would report healthy machines as missing.
  bool _isOnline(Map<String, dynamic> computer) {
    final raw = computer['last_seen_at'] as String?;
    if (raw == null) return false;
    final seen = DateTime.tryParse(raw);
    if (seen == null) return false;
    return DateTime.now().toUtc().difference(seen.toUtc()) <
        const Duration(seconds: 90);
  }

  Future<void> _addComputer() async {
    String? token;
    String? error;
    try {
      token = await _service.createBindingToken();
    } on ApiException catch (e) {
      error = e.message;
    }
    if (!mounted) return;

    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Add a computer'),
        content: error != null
            ? Text(error)
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text(
                    'Install the Guardian agent on the machine and put this token in its '
                    'agent.env as BINDING_TOKEN. The agent trades it for its own credential '
                    'on first run and then deletes it from the file.',
                    style: TextStyle(color: GuardianColors.muted, height: 1.45),
                  ),
                  const SizedBox(height: 14),
                  Container(
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: GuardianColors.sunken,
                      borderRadius: BorderRadius.circular(6),
                      border: Border.all(color: GuardianColors.line),
                    ),
                    child: SelectableText(token!, style: monoStyle(size: 12)),
                  ),
                ],
              ),
        actions: [
          if (error == null)
            TextButton(
              onPressed: () {
                Clipboard.setData(ClipboardData(text: token!));
                Navigator.pop(context);
              },
              child: const Text('Copy'),
            ),
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }

  void _openLog(Map<String, dynamic> computer) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ComputerLogScreen(
          computerId: computer['id'] as String,
          name: computerName(computer),
        ),
      ),
    );
  }

  Future<void> _rename(Map<String, dynamic> computer) async {
    final controller = TextEditingController(
      text: (computer['display_name'] as String?) ?? '',
    );
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Rename computer'),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: InputDecoration(
            labelText: 'Name',
            hintText: (computer['hostname'] as String?) ?? '',
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
    if (ok != true) return;
    await _run(
      () => _service.updateComputer(computer['id'] as String, {
        'display_name': name,
      }),
      'Computer renamed',
    );
  }

  Future<void> _unenroll(Map<String, dynamic> computer) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Remove ${computerName(computer)}?'),
        content: const Text(
          'This revokes that machine\'s credential, so its agent stops syncing. It keeps '
          'enforcing the last policy it received until somebody reinstalls it, and it frees '
          'a seat on your plan.',
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
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await _run(
      () => _service.deleteComputer(computer['id'] as String),
      'Computer removed',
    );
  }

  Future<void> _pickRoom(Map<String, dynamic> computer) async {
    final current = computer['room_id'] as String?;
    final picked = await showModalBottomSheet<(String?,)>(
      context: context,
      builder: (context) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: [
            const SectionLabel('Move to room'),
            for (final (String? id, String name) in [
              (null, 'No room — enforces nothing'),
              for (final r in _rooms) (r['id'] as String, r['name'] as String),
            ])
              ListTile(
                title: Text(name),
                trailing: id == current
                    ? const Icon(Icons.check, color: GuardianColors.guardian)
                    : null,
                onTap: () => Navigator.pop(context, (id,)),
              ),
          ],
        ),
      ),
    );
    if (picked == null || picked.$1 == current) return;
    await _run(
      () => _service.updateComputer(computer['id'] as String, {
        'room_id': picked.$1,
      }),
      picked.$1 == null ? 'Taken out of every room' : 'Moved',
    );
  }

  @override
  Widget build(BuildContext context) {
    final online = _computers.where(_isOnline).length;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Computers'),
        actions: [
          if (_computers.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(right: 4),
              child: Center(
                child: Text(
                  '$online/${_computers.length} online',
                  style: monoStyle(size: 12, color: GuardianColors.muted),
                ),
              ),
            ),
          IconButton(
            icon: const Icon(Icons.add),
            tooltip: 'Add a computer',
            onPressed: _addComputer,
          ),
        ],
      ),
      body: RefreshIndicator(onRefresh: _load, child: _buildBody()),
    );
  }

  Widget _buildBody() {
    if (_loading) return const Center(child: CircularProgressIndicator());
    if (_error != null) {
      return EmptyState(
        icon: Icons.cloud_off,
        title: 'Could not load the computers',
        detail: _error,
      );
    }
    if (_computers.isEmpty) {
      return EmptyState(
        icon: Icons.computer_outlined,
        title: 'No computers yet',
        detail: 'Get an installer token and put it on the machine.',
        action: FilledButton.icon(
          onPressed: _addComputer,
          icon: const Icon(Icons.add),
          label: const Text('Add a computer'),
        ),
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: 8),
      itemCount: _computers.length,
      itemBuilder: (context, index) {
        final computer = _computers[index];
        final roomId = computer['room_id'] as String?;
        final room = _rooms.firstWhere(
          (r) => r['id'] == roomId,
          orElse: () => const {},
        );
        return _ComputerCard(
          computer: computer,
          online: _isOnline(computer),
          roomName: room['name'] as String?,
          onOpenLog: () => _openLog(computer),
          onPickRoom: () => _pickRoom(computer),
          onLock: (v) => _run(
            () => _service.updateComputer(computer['id'] as String, {
              'blocked': v,
            }),
            v ? 'Computer locked' : 'Computer unlocked',
          ),
          onMenu: (choice) {
            if (choice == 'log') _openLog(computer);
            if (choice == 'rename') _rename(computer);
            if (choice == 'remove') _unenroll(computer);
          },
        );
      },
    );
  }
}

class _ComputerCard extends StatelessWidget {
  const _ComputerCard({
    required this.computer,
    required this.online,
    required this.roomName,
    required this.onOpenLog,
    required this.onPickRoom,
    required this.onLock,
    required this.onMenu,
  });

  final Map<String, dynamic> computer;
  final bool online;
  final String? roomName;
  final VoidCallback onOpenLog;
  final VoidCallback onPickRoom;
  final ValueChanged<bool> onLock;
  final ValueChanged<String> onMenu;

  @override
  Widget build(BuildContext context) {
    final blocked = computer['blocked'] as bool? ?? false;
    final hostname = (computer['hostname'] as String?) ?? '';
    final facts = [
      if (hostname.isNotEmpty && hostname != computerName(computer)) hostname,
      (computer['os_name'] as String?) ?? '',
      if ((computer['agent_version'] as String?)?.isNotEmpty ?? false)
        'agent ${computer['agent_version']}',
    ].where((e) => e.isNotEmpty).join(' · ');

    return Card(
      clipBehavior: Clip.antiAlias,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: onOpenLog,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 12, 4, 8),
              child: Row(
                children: [
                  StatusDot(on: online),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            Flexible(
                              child: Text(
                                computerName(computer),
                                style: Theme.of(context).textTheme.titleMedium,
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                            if (blocked) ...[
                              const SizedBox(width: 8),
                              const StateBadge(
                                'Locked',
                                color: GuardianColors.amber,
                              ),
                            ],
                          ],
                        ),
                        const SizedBox(height: 2),
                        Text(
                          online
                              ? 'online'
                              : 'seen ${relativeTime(computer['last_seen_at'] as String?)}',
                          style: monoStyle(
                            size: 12,
                            color: online
                                ? GuardianColors.guardian
                                : GuardianColors.muted,
                          ),
                        ),
                        if (facts.isNotEmpty)
                          Text(
                            facts,
                            style: monoStyle(
                              size: 11,
                              color: GuardianColors.dim,
                            ),
                            overflow: TextOverflow.ellipsis,
                          ),
                      ],
                    ),
                  ),
                  PopupMenuButton<String>(
                    icon: const Icon(
                      Icons.more_vert,
                      color: GuardianColors.muted,
                    ),
                    onSelected: onMenu,
                    itemBuilder: (context) => const [
                      PopupMenuItem(value: 'log', child: Text('Blocking log')),
                      PopupMenuItem(value: 'rename', child: Text('Rename')),
                      PopupMenuItem(
                        value: 'remove',
                        child: Text(
                          'Remove from account',
                          style: TextStyle(color: GuardianColors.danger),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          const Divider(),
          Row(
            children: [
              Expanded(
                child: InkWell(
                  onTap: onPickRoom,
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(16, 10, 8, 10),
                    child: Row(
                      children: [
                        const Icon(
                          Icons.meeting_room_outlined,
                          size: 18,
                          color: GuardianColors.muted,
                        ),
                        const SizedBox(width: 8),
                        Flexible(
                          child: Text(
                            roomName ?? 'No room',
                            style: TextStyle(
                              color: roomName == null
                                  ? GuardianColors.amber
                                  : GuardianColors.ink,
                            ),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        const Icon(
                          Icons.expand_more,
                          size: 18,
                          color: GuardianColors.muted,
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              const Text(
                'Lock',
                style: TextStyle(color: GuardianColors.muted, fontSize: 13),
              ),
              const SizedBox(width: 4),
              Padding(
                padding: const EdgeInsets.only(right: 8),
                child: Switch(
                  value: blocked,
                  // Locked is a state that needs attention, not "enforcing".
                  activeTrackColor: GuardianColors.amber,
                  trackOutlineColor: WidgetStateProperty.resolveWith(
                    (s) => s.contains(WidgetState.selected)
                        ? GuardianColors.amber
                        : GuardianColors.line,
                  ),
                  onChanged: onLock,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
