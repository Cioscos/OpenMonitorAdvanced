using System.Text.Json;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Every <see cref="CanonicalNames.LabelKeys"/> entry must exist as <c>sensor.&lt;key&gt;</c>
/// in both i18n catalogs. Finds the repo root by walking up from the test assembly's
/// directory looking for <c>app/src/lib/i18n/en.json</c>.
/// </summary>
public sealed class LabelKeyTests
{
    private static string RepoRoot()
    {
        DirectoryInfo? dir = new(AppContext.BaseDirectory);
        while (dir is not null)
        {
            if (File.Exists(Path.Combine(dir.FullName, "app", "src", "lib", "i18n", "en.json")))
            {
                return dir.FullName;
            }

            dir = dir.Parent;
        }

        throw new InvalidOperationException("Could not find the repo root (app/src/lib/i18n/en.json) above " + AppContext.BaseDirectory);
    }

    private static IReadOnlyDictionary<string, JsonElement> LoadCatalog(string fileName)
    {
        string path = Path.Combine(RepoRoot(), "app", "src", "lib", "i18n", fileName);
        using JsonDocument document = JsonDocument.Parse(File.ReadAllText(path));
        var map = new Dictionary<string, JsonElement>();
        foreach (JsonProperty property in document.RootElement.EnumerateObject())
        {
            map[property.Name] = property.Value.Clone();
        }

        return map;
    }

    [Fact]
    public void EveryLabelKeyHasATranslationInBothCatalogs()
    {
        IReadOnlyDictionary<string, JsonElement> en = LoadCatalog("en.json");
        IReadOnlyDictionary<string, JsonElement> it = LoadCatalog("it.json");

        Assert.NotEmpty(CanonicalNames.LabelKeys);

        foreach (string key in CanonicalNames.LabelKeys)
        {
            string wireKey = $"sensor.{key}";
            Assert.True(en.ContainsKey(wireKey), $"Missing '{wireKey}' in en.json");
            Assert.True(it.ContainsKey(wireKey), $"Missing '{wireKey}' in it.json");
        }
    }
}
