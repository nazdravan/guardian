import 'package:flutter/material.dart';

import '../services/settings_service.dart';
import '../theme.dart';
import '../utils/snackbar_helper.dart';

/// Sign in, or create an account. The app used to be gated by a shared admin
/// token typed into Settings — one secret, no identity, no way to revoke one
/// phone without revoking every phone. It signs in as a person now.
class LoginScreen extends StatefulWidget {
  const LoginScreen({super.key});

  @override
  State<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends State<LoginScreen> {
  final SettingsService _service = SettingsService();
  final _serverController = TextEditingController();
  final _emailController = TextEditingController();
  final _passwordController = TextEditingController();
  final _nameController = TextEditingController();

  bool _registering = false;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _serverController.text = _service.serverAddress;
  }

  @override
  void dispose() {
    _serverController.dispose();
    _emailController.dispose();
    _passwordController.dispose();
    _nameController.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final server = _serverController.text.trim();
    final email = _emailController.text.trim();
    final password = _passwordController.text;

    if (!server.startsWith('http://') && !server.startsWith('https://')) {
      showSnackBarMessage(
        context,
        message: 'The server address must start with http:// or https://',
        kind: MessageKind.warning,
      );
      return;
    }
    if (email.isEmpty || password.isEmpty) {
      showSnackBarMessage(
        context,
        message: 'Email and password are required',
        kind: MessageKind.warning,
      );
      return;
    }

    setState(() => _busy = true);
    try {
      await _service.setServerAddress(server);
      if (_registering) {
        await _service.register(email, password, _nameController.text.trim());
      }
      await _service.signIn(email, password);
      // No navigation here: the root widget watches the service and swaps the
      // screen once a session exists.
    } on ApiException catch (e) {
      if (mounted) {
        showSnackBarMessage(
          context,
          message: e.message,
          kind: MessageKind.error,
        );
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Icon(
                    Icons.shield,
                    size: 56,
                    color: GuardianColors.guardian,
                  ),
                  const SizedBox(height: 14),
                  Text(
                    'Guardian',
                    textAlign: TextAlign.center,
                    style: Theme.of(context).textTheme.headlineMedium,
                  ),
                  const SizedBox(height: 6),
                  Text(
                    _registering
                        ? 'Create the account your computers will join.'
                        : 'Sign in to manage your rooms and computers.',
                    textAlign: TextAlign.center,
                    style: const TextStyle(color: GuardianColors.muted),
                  ),
                  const SizedBox(height: 28),
                  TextField(
                    controller: _serverController,
                    style: monoStyle(size: 14),
                    decoration: const InputDecoration(
                      labelText: 'Server address',
                      hintText: 'https://guardian.example.com',
                    ),
                    keyboardType: TextInputType.url,
                    autocorrect: false,
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    controller: _emailController,
                    decoration: const InputDecoration(labelText: 'Email'),
                    keyboardType: TextInputType.emailAddress,
                    autocorrect: false,
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    controller: _passwordController,
                    decoration: const InputDecoration(labelText: 'Password'),
                    obscureText: true,
                  ),
                  if (_registering) ...[
                    const SizedBox(height: 12),
                    TextField(
                      controller: _nameController,
                      decoration: const InputDecoration(
                        labelText: 'Your name (optional)',
                      ),
                    ),
                  ],
                  const SizedBox(height: 20),
                  FilledButton(
                    style: FilledButton.styleFrom(
                      minimumSize: const Size.fromHeight(48),
                    ),
                    onPressed: _busy ? null : _submit,
                    child: _busy
                        ? const SizedBox(
                            height: 20,
                            width: 20,
                            child: CircularProgressIndicator(
                              strokeWidth: 2,
                              color: GuardianColors.void_,
                            ),
                          )
                        : Text(_registering ? 'Create account' : 'Sign in'),
                  ),
                  TextButton(
                    onPressed: _busy
                        ? null
                        : () => setState(() => _registering = !_registering),
                    child: Text(
                      _registering
                          ? 'I already have an account'
                          : 'Create a new account',
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
