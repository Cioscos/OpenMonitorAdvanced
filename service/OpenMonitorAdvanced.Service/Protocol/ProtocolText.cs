namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>Bounds client-supplied text before it is echoed back in an error message or a log.</summary>
internal static class ProtocolText
{
    /// <summary>
    /// <paramref name="text"/> with every control character replaced by <c>?</c> and cut to at
    /// most <paramref name="max"/> characters plus an ellipsis; a surrogate pair is never split.
    /// <c>null</c> is the empty string.
    /// </summary>
    internal static string Clip(string? text, int max = 64)
    {
        if (string.IsNullOrEmpty(text))
        {
            return "";
        }

        bool clipped = text.Length > max;
        int length = clipped ? max : text.Length;
        if (clipped && char.IsHighSurrogate(text[length - 1]))
        {
            length--;
        }

        return string.Create(length + (clipped ? 1 : 0), (text, length, clipped), static (span, state) =>
        {
            for (int i = 0; i < state.length; i++)
            {
                char c = state.text[i];
                span[i] = char.IsControl(c) ? '?' : c;
            }

            if (state.clipped)
            {
                span[^1] = '…';
            }
        });
    }
}
