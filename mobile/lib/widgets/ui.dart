import 'package:flutter/material.dart';

import '../theme.dart';

/// A whole-screen message: nothing here yet, or something went wrong. Lives in
/// a ListView so pull-to-refresh still works when there is nothing to pull.
class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    this.detail,
    this.action,
  });

  final IconData icon;
  final String title;
  final String? detail;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 96),
      children: [
        Icon(icon, size: 40, color: GuardianColors.dim),
        const SizedBox(height: 16),
        Text(
          title,
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.titleMedium,
        ),
        if (detail != null) ...[
          const SizedBox(height: 8),
          Text(
            detail!,
            textAlign: TextAlign.center,
            style: const TextStyle(color: GuardianColors.muted, height: 1.45),
          ),
        ],
        if (action != null) ...[
          const SizedBox(height: 20),
          Center(child: action),
        ],
      ],
    );
  }
}

/// A small label in a state colour. [color] null means greyscale: the thing
/// is a fact, not a state.
class StateBadge extends StatelessWidget {
  const StateBadge(this.label, {super.key, this.color});

  final String label;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final c = color ?? GuardianColors.muted;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: c.withValues(alpha: color == null ? 0.08 : 0.12),
        borderRadius: BorderRadius.circular(4),
        border: Border.all(color: c.withValues(alpha: 0.35)),
      ),
      child: Text(
        label,
        style: TextStyle(
          fontFamily: GuardianFonts.ui,
          fontSize: 12,
          fontWeight: FontWeight.w500,
          color: color ?? GuardianColors.muted,
        ),
      ),
    );
  }
}

/// The dot in front of a machine: green when online, dim otherwise.
class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.on, this.color});

  final bool on;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final c = on ? (color ?? GuardianColors.guardian) : GuardianColors.dim;
    return Container(
      width: 8,
      height: 8,
      decoration: BoxDecoration(
        color: c,
        shape: BoxShape.circle,
        boxShadow: on
            ? [BoxShadow(color: c.withValues(alpha: 0.5), blurRadius: 6)]
            : null,
      ),
    );
  }
}

/// A small upper-case caption above a group of rows.
class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key, this.trailing});

  final String text;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 20, 16, 8),
      child: Row(
        children: [
          Expanded(
            child: Text(
              text.toUpperCase(),
              style: const TextStyle(
                fontFamily: GuardianFonts.ui,
                fontSize: 11,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.8,
                color: GuardianColors.muted,
              ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

/// An inline banner for a failed load, with the server's own words.
class ErrorBanner extends StatelessWidget {
  const ErrorBanner(this.message, {super.key, this.onRetry});

  final String message;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 12, 12, 0),
      padding: const EdgeInsets.fromLTRB(14, 10, 6, 10),
      decoration: BoxDecoration(
        color: GuardianColors.danger.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
          color: GuardianColors.danger.withValues(alpha: 0.35),
        ),
      ),
      child: Row(
        children: [
          const Icon(
            Icons.error_outline,
            size: 18,
            color: GuardianColors.danger,
          ),
          const SizedBox(width: 10),
          Expanded(child: Text(message)),
          if (onRetry != null)
            TextButton(onPressed: onRetry, child: const Text('Retry')),
        ],
      ),
    );
  }
}
