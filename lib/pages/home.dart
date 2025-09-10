import 'package:flutter/material.dart';
import 'package:flutter_form_builder/flutter_form_builder.dart';
import 'package:gap/gap.dart';
import 'package:lwdproxy/src/rust/api/proxy.dart';

class HomePage extends StatefulWidget {
  @override
  State<StatefulWidget> createState() => HomePageState();
}

class HomePageState extends State<HomePage> {
  final formKey = GlobalKey<FormBuilderState>();

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: Text("LWD Proxy")),
      body: SingleChildScrollView(
        child: Padding(
          padding: EdgeInsetsGeometry.symmetric(horizontal: 16),
          child: FormBuilder(
            key: formKey,
            child: Column(
              children: [
                FormBuilderTextField(
                  name: "origin",
                  decoration: InputDecoration(label: Text("Origin URL")),
                  initialValue: "https://zec.rocks",
                ),
                Gap(32),
                ElevatedButton.icon(onPressed: onStart, label: Text("Start")),
              ],
            ),
          ),
        ),
      ),
    );
  }

  void onStart() async {
    final form = formKey.currentState!;
    final fields = form.fields;
    final origin = fields["origin"]!.value as String;
    await startProxy(origin: origin);
    // await startServer(bindAddress: "0.0.0.0", port: 9067);
  }
}
