fn horizonvc_register(commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    for (usage, level) in horizonvc_core::USAGES {
        commands.push(crate::game::native_server::NativeCommand::op(usage, level));
    }
}
