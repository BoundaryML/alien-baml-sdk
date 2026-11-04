"""Calls through the facade that mypy checks (test_facade.py runs it on this
file, with --warn-unused-ignores); never run. Each ``type: ignore`` marks a
call mypy must reject, so the file only checks clean when mypy sees the
operations' real signatures."""

from __future__ import annotations

from typing_extensions import assert_type

from alien_sdk import Alien, AlienManager, AsyncAlien, AsyncAlienManager, manager, platform

alien = Alien()
reveal_type(alien.deployments.list)
assert_type(alien.deployments.list(project="web", deployment_group="prod", limit=10), platform.ListDeploymentsResponse)
assert_type(alien.managers.get("mgr_1"), platform.Manager)
assert_type(alien.accept_workspace_invitation("token"), platform.AcceptWorkspaceInvitationResponse)
alien.deployments.list(deploymentGroup="prod")  # type: ignore[call-arg]
alien.deployments.list(deployment_grop="prod")  # type: ignore[call-arg]
alien.deployments.list(limit="10")  # type: ignore[arg-type]
alien.managers.get()  # type: ignore[call-arg]
alien.deployments.lst()  # type: ignore[attr-defined]

commands = AlienManager("http://manager").commands
assert_type(commands.get_command_status("cmd_1"), manager.CommandStatusResponse)


async def asynchronous() -> None:
    a = AsyncAlien()
    reveal_type(a.deployments.list)
    assert_type(await a.deployments.list(deployment_group="prod"), platform.ListDeploymentsResponse)
    await a.deployments.list(deploymentGroup="prod")  # type: ignore[call-arg]
    m = AsyncAlienManager("http://manager")
    assert_type(await m.commands.get_command_status("cmd_1"), manager.CommandStatusResponse)
