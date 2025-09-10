import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:lwdproxy/pages/home.dart';
import 'package:lwdproxy/src/rust/frb_generated.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp.router(
      routerConfig: router,
      debugShowCheckedModeBanner: false,
    );
  }
}

final router = GoRouter(
  routes: [GoRoute(path: "/", builder: (context, state) => HomePage())],
);
