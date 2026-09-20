using UnityEngine;

/// <summary>
///     Sits on Qux, the deepest active object. That way a reader walking
///     the hierarchy finds a component with a managed side and a value to
///     check. It has its own file because Unity maps a behaviour to a script
///     by file name.
/// </summary>
public class Marker : MonoBehaviour
{
    public int value;

    void Awake()
    {
        value = 4242;
    }
}
