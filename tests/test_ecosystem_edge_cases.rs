mod common;
use common::TestRepo;

#[test]
fn test_elixir_prerelease_version() {
    let repo = TestRepo::new();

    let mix_exs = r#"defmodule MyApp.MixProject do
  use Mix.Project

  def project do
    [
      app: :my_app,
      version: "2.0.0-beta.1",
      elixir: "~> 1.14",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  defp deps, do: []
end
"#;

    repo.write_file("mix.exs", mix_exs);
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Elixir prerelease version should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("my_app"),
        "Elixir app should be detected"
    );
    assert!(
        bootstrap.contains("2.0.0-beta.1") || bootstrap.contains("2.0.0"),
        "Version should be captured"
    );
}

#[test]
fn test_elixir_missing_version() {
    let repo = TestRepo::new();

    let mix_exs = r#"defmodule NoVersion.MixProject do
  use Mix.Project

  def project do
    [
      app: :no_version_app,
      elixir: "~> 1.14"
    ]
  end
end
"#;

    repo.write_file("mix.exs", mix_exs);
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Missing version should default gracefully: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("no_version_app"),
        "App should be detected even without version"
    );
}

#[test]
fn test_go_github_module() {
    let repo = TestRepo::new();

    let go_mod = r"module github.com/organization/project

go 1.21

require (
    github.com/gin-gonic/gin v1.9.0
)
";

    repo.write_file("go.mod", go_mod);
    repo.write_file("main.go", "package main\n\nfunc main() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "GitHub module should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("github.com/organization/project"),
        "Full module path should be captured"
    );
}

#[test]
fn test_go_vanity_url_module() {
    let repo = TestRepo::new();

    let go_mod = r"module gopkg.in/yaml.v3

go 1.20
";

    repo.write_file("go.mod", go_mod);
    repo.write_file("yaml.go", "package yaml\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Vanity URL module should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("gopkg.in/yaml.v3"),
        "Vanity URL should be captured"
    );
}

#[test]
fn test_go_workspace() {
    let repo = TestRepo::new();

    let go_work = r"go 1.21

use (
    ./cmd/app
    ./pkg/lib
)
";

    let app_mod = r"module myproject/cmd/app

go 1.21
";

    let lib_mod = r"module myproject/pkg/lib

go 1.21
";

    repo.write_file("go.work", go_work);
    repo.write_file("cmd/app/go.mod", app_mod);
    repo.write_file("cmd/app/main.go", "package main\n\nfunc main() {}\n");
    repo.write_file("pkg/lib/go.mod", lib_mod);
    repo.write_file("pkg/lib/lib.go", "package lib\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Go workspace should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("myproject/cmd/app") || bootstrap.contains("app"),
        "App module should be detected"
    );
    assert!(
        bootstrap.contains("myproject/pkg/lib") || bootstrap.contains("lib"),
        "Lib module should be detected"
    );
}

#[test]
fn test_go_simple_module() {
    let repo = TestRepo::new();

    let go_mod = r"module example

go 1.21
";

    repo.write_file("go.mod", go_mod);
    repo.write_file("main.go", "package main\n\nfunc main() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Simple module name should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("example"),
        "Simple module should be detected"
    );
}

#[test]
fn test_mixed_elixir_and_go() {
    let repo = TestRepo::new();

    let mix_exs = r#"defmodule WebApp.MixProject do
  use Mix.Project

  def project do
    [
      app: :web_app,
      version: "1.0.0"
    ]
  end
end
"#;

    let go_mod = r"module github.com/org/backend

go 1.21
";

    repo.write_file("frontend/mix.exs", mix_exs);
    repo.write_file("backend/go.mod", go_mod);
    repo.write_file("backend/main.go", "package main\n\nfunc main() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Mixed Elixir and Go should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("web_app"),
        "Elixir app should be detected"
    );
    assert!(
        bootstrap.contains("github.com/org/backend") || bootstrap.contains("backend"),
        "Go module should be detected"
    );
}

#[test]
fn test_elixir_with_dynamic_version() {
    let repo = TestRepo::new();

    let mix_exs = r#"defmodule DynamicVersion.MixProject do
  use Mix.Project

  @version "1.2.3"

  def project do
    [
      app: :dynamic_app,
      version: @version,
      elixir: "~> 1.14"
    ]
  end
end
"#;

    repo.write_file("mix.exs", mix_exs);
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Dynamic version should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("dynamic_app"),
        "App should be detected even with dynamic version"
    );
}

#[test]
fn test_go_with_replace_directive() {
    let repo = TestRepo::new();

    let go_mod = r"module myproject

go 1.21

require (
    github.com/external/dep v1.0.0
)

replace github.com/external/dep => ../local-dep
";

    repo.write_file("go.mod", go_mod);
    repo.write_file("main.go", "package main\n\nfunc main() {}\n");
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Replace directive should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(bootstrap.contains("myproject"), "Module should be detected");
}

#[test]
fn test_elixir_phoenix_project() {
    let repo = TestRepo::new();

    let mix_exs = r#"defmodule MyPhoenixApp.MixProject do
  use Mix.Project

  def project do
    [
      app: :my_phoenix_app,
      version: "0.1.0",
      elixir: "~> 1.14",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      aliases: aliases(),
      deps: deps()
    ]
  end

  def application do
    [
      mod: {MyPhoenixApp.Application, []},
      extra_applications: [:logger, :runtime_tools]
    ]
  end

  defp elixirc_paths(:test), do: ["lib", "test/support"]
  defp elixirc_paths(_), do: ["lib"]

  defp deps do
    [
      {:phoenix, "~> 1.7.0"},
      {:phoenix_ecto, "~> 4.4"},
      {:ecto_sql, "~> 3.6"},
      {:postgrex, ">= 0.0.0"}
    ]
  end

  defp aliases do
    [
      setup: ["deps.get", "ecto.setup"],
      "ecto.setup": ["ecto.create", "ecto.migrate", "run priv/repo/seeds.exs"],
      "ecto.reset": ["ecto.drop", "ecto.setup"],
      test: ["ecto.create --quiet", "ecto.migrate --quiet", "test"]
    ]
  end
end
"#;

    repo.write_file("mix.exs", mix_exs);
    repo.commit("initial commit");

    let output = repo.run_belaf_command(&["init", "--force"]);

    assert!(
        output.status.success(),
        "Phoenix project should be handled: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bootstrap = repo.read_file("belaf/bootstrap.toml");
    assert!(
        bootstrap.contains("my_phoenix_app"),
        "Phoenix app should be detected"
    );
    assert!(bootstrap.contains("0.1.0"), "Version should be captured");
}
