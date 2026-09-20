// The oldest editors compile this with their C# 4.0 compiler, so the file
// stays inside what every supported editor accepts.
using System.Collections.Generic;
using UnityEngine;
using UnityEngine.SceneManagement;

/// <summary>
///     Holds the values unity-fixture.json describes. Everything is set once
///     in Awake and never changes, so a run reads the same at any point.
/// </summary>
public class FixtureData : MonoBehaviour
{
    public static FixtureData Instance;
    public static int StaticInt;
    public static string StaticString;

    public int intValue;
    public long longValue;
    public float floatValue;
    public double doubleValue;
    public bool boolValue;
    public string stringValue;
    public Hero hero;
    public int[] intArray;
    public List<int> intList;
    public List<string> stringList;
    public Dictionary<string, int> dict;

    void Awake()
    {
        Instance = this;
        StaticInt = 4242;
        StaticString = "static-fixture";

        intValue = 1337;
        longValue = 9000000000L;
        floatValue = 3.5f;
        doubleValue = 2.718281828;
        boolValue = true;
        stringValue = "hello-fixture";
        hero = new Hero();
        hero.id = 7;
        hero.level = 12;
        hero.title = "the-brave";
        intArray = new int[] { 100, 200, 300 };
        intList = new List<int> { 10, 20, 30 };
        stringList = new List<string> { "a", "b", "c" };
        dict = new Dictionary<string, int> { { "one", 1 }, { "two", 2 }, { "three", 3 } };

        // The object survives scene loads. That way the scene Unity keeps for
        // such objects has one root to read.
        DontDestroyOnLoad(gameObject);

        // Loads a second scene so a reader sees more than one.
        SceneManager.LoadScene("Second", LoadSceneMode.Additive);
    }
}

public class Hero
{
    public int id;
    public int level;
    public string title;
}
