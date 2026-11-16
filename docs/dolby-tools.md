# Dolby tool command lines on the development machine

Captured 2026-09-09 by `tools/capture_dolby_help.py`. These outputs
are the ground truth for the flags used by the verification scripts.
Notes below the marker at the end are written by hand and are kept.

## `dee.exe --help`

```text
dee.exe, Version 5.2.1, Jun 10 2022
Options:
  --help, -h                               : Print this help.
  --xml, -x FILE                           : Specify a file with XML profile.
  --json, -j FILE                          : Specify a file with JSON profile.
  --schema, -s FILE                        : Specify a file to write XML schema.
  --input-audio, -a FILE                   : Specify a file with audio source. Not required if already provided in XML profile.
  --input-video, -v FILE                   : Specify a file with video source. Not required if already provided in XML profile.
  --output, -o FILE                        : Specify a file to write the output. Not required if already provided in XML profile.
  --log-file FILE                          : Specify a log file.
  --stdout                                 : If a log file is specified this option duplicates the log to standard output. Otherwise this option is ignored.
  --verbose LEVEL                          : LEVEL values are: quiet, normal, info, debug (default)
  --license-file, -l FILE                  : Specify a license file.
  --trace-file FILE                        : Specify a trace file.
  --trace-length NUMBER                    : Number of frames to keep in trace buffer (default=100000). 
  --progress                               : Enable progress bar. Ignored when no log file specified.
  --progress-interval MILLISECONDS         : Set progress reporting interval (default=1000).
  --diagnostics-interval MILLISECONDS      : Set diagnostics (e.g. MEM/CPU usage) reporting interval (default=1000).
  --add-elem                               : Add (overwrite if exists) XML element.
  --disable-xml-validation                 : Disable xml validation.
  --print-stages                           : Print available stages. Those not covered by license will be marked as disabled.
  --temp DIR                               : Set temp_dir to specified directory. This option overrides temp_dir specified in XML profile.
  --ignore-logs STRING                     : Ignore logs containing specified string or at least one of strings from comma-sperated list.

Usage Examples:
1. Write XML Schema to a file:
   dee.exe --schema C:\Users\CurrentUser\schema.xsd
2. Launch encoding job with progress reporting:
   dee.exe --progress --log-file log.log --xml ..\mxf_dv_mezz_to_dv_profile_5_hevc.xml
3. Launch encoding job with input from the CLI (overwrite input provided in XML if exists):
   dee.exe --xml ..\mxf_dv_mezz_to_dv_profile_5_hevc.xml --input-video C:\Video\Movie\movie.mxf
   dee.exe --xml ..\damf_atmos_mezz_to_atmos_ddp_ec3.xml --input-audio ..\Audio\printmaster.atmos


Time elapsed: 0.51158 seconds
```

## `dee.exe --print-stages`

```text
[2026-09-09 19:48:54.635] INFO: Dolby Encoding Engine, version: 5.2.1-5994839.
Audio inputs:
               ac4
               wav
               wav_list
               pcm_s16le
               pcm_s24le
               pcm_f32le
               adm
               damf
               wav_s337m
               ac3
               ec3
               dde
               generic
               atmos_mezz
               mxf_iab
               mp4
               manifest
               ts

Video inputs:
               mxf
               mxf_sidecar
               j2k_sidecar
               tiff_sidecar
               mov
               mov_sidecar
               hevc
               avc
               mp4
               ts
               bl_el_rpu
               yuv420
               yuv420p10le
               yuv420p12le
               rgb48le
               gbrp16le
               rgbp16le
               yuv420p16le
               dvmd
               prores_sidecar
               manifest
               j2k_stream

Audio filters:
               pcm_channel_order
               convert_atmos_mezz
               pcm_to_ddp
               pcm_loudness_correction
               encode_to_atmos_ddp
               atmos_mezz_to_atmos_ddp
               pcm_to_atmos_ddp
               ddp_decode
               pcm_loudness_correction_single_pass
               encode_to_dd_single_pass
               measure_loudness
               transcode_to_ddp
               encode_to_dthd
               edit_ddp

Video filters:
               sdr_to_dv_profile_4_2
               dv_mezz_to_dv_profile_5
               dv_mezz_preproc_profile_5
               extract_j2k
               dv_md_postproc
               dv_ves_mux
               dv_mezz_to_dv_profile_7_el
               dv_mezz_to_dv_profile_7_bl
               dv_mezz_to_dv_profile_8_1
               dv_mezz_to_dv_profile_8_1_yuv
               sdr_to_dv_profile_8_2
               dv_mezz_to_hdr10
               dv_mezz_to_hdr10_yuv
               dv_mezz_to_sdr
               dv_mezz_to_sdr_yuv
               extract_prores
               encode_to_hevc
               transcode_to_hevc
               parse_mxf

Outputs:
               ac3
               ac4
               aac
               mlp
               wav
               ec3
               hevc
               yuv420_rpu
               generic
               mp4
               ts
               j2k_sidecar
               prores
               atmos_mezz
               manifest
               yuv420p10le
               yuv420p12le
               yuv420p16le
               dvmd
               pcm_s16le
               pcm_s24le
               pcm_f32le
               dvsd
               dvrpu

Time elapsed: 2.79269 seconds
```

## `atmos_info.exe --help`

```text
AtmosInfo Tool (version 1.1)
Options:
	--input, -i FILE        : specify input atmos file
	--skip-validation, -s   : skip validation step for ADM BWF input
	--help, -h              : print help
	--version, -v           : print application and library version
```

## `bwf_info.exe --help`

```text
Broadcast Wave Format Info.
This tool belongs to the Dolby Encoding Engine version 5.2.1
Interface version: 0.9.0 (Jun 10 2022):
  -h [ --help ]          Show this help.
  --loglevel arg (=info) Logging level followed by log customization options. Use "--morehelp 
                         loglevel" for more details. Values: debug|error|info|quiet|warning.
  -i [ --input ] arg     Input BWF file.
  --morehelp arg         Show more help about selected topic. Values: loglevel.


Execution time: 0.0004066 seconds
Exit code: 0
```

## `dee_dthd_encoder.exe --help`

```text
Dolby TrueHD with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.0.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --presentation arg                    Dolby TrueHD presentation configuration. Values: 
                                        2ch|6ch|8ch|atmos. Use "--morehelp presentation" for more 
                                        details.
  -f [ --input-format ] arg             Presentation's input format. Positional argument for option
                                        "presentation". Use "--morehelp input-format" for more 
                                        details. Values: atmos_mezz|wav|wav_list.
  -i [ --input ] arg                    Input file for presentation. Positional argument for option
                                        "presentation".
  -o [ --output ] arg                   Output file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample".
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file".
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only|skip.
  --optimize-data-rate arg (=0)         Enables additional preprocessing that allows reduction of 
                                        peak data rate.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        add-silence|embed-timecode|examples|extras|input-format|lou
                                        dness-management|presentation|loglevel|all.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


Execution time: 0.0007 seconds
Exit code: 0
```

## `dee_dthd_encoder.exe --morehelp input-format`

```text
Option 'input-format' is a positional parameter, which is assigned to the last preceding DolbyTrueHD presentation specified by '--presentation' option.

Formats available for '--presentation' 'atmos':
  Single Dolby Atmos file (DAMF, BWF ADM or MXF IAB.
  For '--presentation' 'atmos', this format is automatically assumed.
  Example:
    --presentation atmos --input-format atmos_mezz --input file.atmos
    is equivalent to
    --presentation atmos --input file.atmos


Formats available for '--presentation' '2ch', '6ch' or '8ch':
  Single 7.1, 5.1 or 2.0 WAVE file input:
  Example:
    --input-format wav --input file.wav

  A list of mono WAVE files, one for each channel, in order of L:R:C:LFE:LS:RS:LRS:RRS, L:R:C:LFE:LS:RS or L:R. Files can be omitted with '-' to be replaced with silence:
  Example: 
    5.1 input with C and LS filled with silence:
    --input-format wav_list --input L.wav:R.wav:-:LFE.wav:-:RS.wav



Execution time: 0.0005992 seconds
Exit code: 0
```

## `dee_dthd_encoder.exe --morehelp presentation`

```text
Option 'presentation' defines to which Dolby TrueHD presentation the next 'input' and 'input-format' options are assigned.
Can be followed by the list of presentation-specific options for further configuration.
This option can be used without corresponding input to configure a downmix presentation.

  Options for 'atmos' presentation:
    drc_profile=<STRING>         (default: film_light). Dynamic range control preset. Values: film_light|film_standard|music_light|music_standard|not_indicated|speech.
    number_of_elements=<INTEGER> (default: 12). Number of clustered signals of Dolby Atmos material to be output by spatial coding. Higher value gives better quality, but increases bitstream size. Values: 12|14|16.
    legacy_authoring=<BOOLEAN>   (default: 1). Make output bitstream compatible with legacy authoring tools.

  Options for '8ch' presentation:
    drc_profile=<STRING>                 (default: film_light). Dynamic range control preset. Values: film_light|film_standard|music_light|music_standard|not_indicated|speech.
    surround_3dB_attenuation=<BOOLEAN>   (default: auto). Controls whether surround channels should be attenuated by 3 dB.
                                         Value 'auto' corresponds to '0' if a presentation is a downmix of 'atmos' presentation, '1' otherwise.

  Options for '6ch' presentation:
    drc_profile=<STRING>                 (default: film_light). Dynamic range control preset. Values: film_light|film_standard|music_light|music_standard|not_indicated|speech.
    surround_3dB_attenuation=<BOOLEAN>   (default: auto). Controls whether surround channels should be attenuated by 3 dB.
                                         Value 'auto' corresponds to '0' if a presentation is a downmix of 'atmos' presentation, '1' otherwise.

  Options for '2ch' presentation:
    drc_profile=<STRING>         (default: film_light). Dynamic range control preset Values: film_light|film_standard|music_light|music_standard|not_indicated|speech.
    drc_default_on=<BOOLEAN>     (default: 1). Setting this to '0' disables DRC for 2 channel presentation. Otherwise, if not specified manually, the default DRC profile is assigned.
    format=<STRING>              (default: stereo). Defines the format of the 2 channel independent presentation. Applicable only for encoding from independent source files. Values: headphone_encoded|stereo|surround_encoded.

Examples:
  Providing input for Dolby Atmos presentation:
    --presentation atmos --input /path/to/input.atmos

  Providing input for both Dolby Atmos and 2 channel presentations:
    --presentation atmos --input /path/to/input.atmos --presentation 2ch --input-format wav --input /path/to/input.wav

  Specifying additional options for independent presentations (Dolby Atmos and 6 channel):
    --presentation atmos:legacy_authoring=0 --input /path/to/input.atmos --presentation 6ch:surround_3db_attenuation=1 --input-format wav --input /path/to/input.wav

  Specifying additional options for downmix presentation (2 channel):
    --presentation 2ch:drc_profile=music_light --presentation atmos --input /path/to/input.atmos


Execution time: 0.0006449 seconds
Exit code: 0
```

## `dee_ddp_encoder.exe --help`

```text
Dolby Digital Plus encoder
This tool belongs to the Dolby Encoding Engine version 5.2.1
Interface version: 0.1.0 (Jun 10 2022):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --cc arg (=1)                         Enable concurrent processing. Use "--morehelp cc" for more 
                                        details. Values: 0|1.
  --input-format arg                    Input format followed by format-specific options. Use 
                                        "--morehelp input-format" for more details. Values: 
                                        wav|wav_list|pcm_s16le|pcm_s24le|pcm_f32le.
  -i [ --input ] arg                    Input file or files, in case of multi-file input.
  --input-manifest arg                  Optional input loudness manifest file.
  -o [ --output ] arg                   Output bitstream file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=till_the_end)        Duration in a timecode or seconds. Values '-1' and 
                                        'till_the_end' mean "up to the last sample".
  --end arg (=end_of_file)              End time given as a timecode, seconds (decimal number) or 
                                        the keyword 'end_of_file'.
  --timecode-frame-rate arg (=not_indicated)
                                        Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --ffoa arg (=auto)                    First frame of action given as a timecode or seconds 
                                        (decimal number). 'auto' means "use input file's FFOA".
  --offset arg (=auto)                  Offset given as a timecode or seconds (decimal number). 
                                        'auto' means "use input file's offset".
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value'. Both values can
                                        be provided as either:
                                        seconds (decimal number)
                                        or
                                        'xf', where x is a number of frames (requires 
                                        timecode-frame-rate to be provided).
  --data-rate arg (=auto)               Target data rate in kbps.. Use "--morehelp data-rate" for 
                                        more details.
  --loudness-management arg (=measure_and_correct)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_and_correct|measure_only|skip.
  -c [ --output-channel-layout ] arg (=auto)
                                        Audio output channel configuration. Values: 
                                        auto|mono|stereo|5.1.
  -e [ --encoder ] arg                  Encoder configuration. Use "--morehelp encoder" for more 
                                        details.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        cc|data-rate|embed-timecode|encoder|input-format|loudness-m
                                        anagement|loglevel.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


Execution time: 0.0007274 seconds
Exit code: 0
```

## `dee_ddp_encoder.exe --morehelp input-format`

```text
Argument of option '--input-format' can be followed by the list of format-specific options.

Single multichannel WAVE file input:
  Example: --input-format wav --input file.wav
  Options: NONE

A list of mono WAVE files, one for each channel, in order of L:R:C:LFE:LS:RS:LRS:RRS. Files can be omitted with '-' to be replaced with silence:
  Examples: 
    Stereo input: --input-format wav_list --input L.wav:R.wav
    5.1 input with C and LS filled with silence: --input-format wav_list --input L.wav:R.wav:-:LFE.wav:-:RS.wav
  Options: NONE

Raw PCM:
  Can be specified as one of the following formats:
    pcm_s16le                 signed 16-bit integer, little-endian
    pcm_s24le                 signed 24-bit integer, little-endian
    pcm_f32le                 32-bit floating point, little-endian
  Example: --input-format pcm_s16le:sample_rate=48000:channels=6
  Options:
    sample_rate=<INTEGER>     Required.
    channels=<INTEGER>        Required.
    streaming=<BOOLEAN>       Optional (default: 0).
    sample_count=<INTEGER>    Optional (default: -1). Value '-1' means "unspecified".




Execution time: 0.0005722 seconds
Exit code: 0
```

## `dee_ddp_encoder.exe --help`

```text
Dolby Digital Plus encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.1.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --cc arg (=1)                         Enable concurrent processing. Use "--morehelp cc" for more 
                                        details. Values: 0|1.
  --input-format arg                    Input format followed by format-specific options. Use 
                                        "--morehelp input-format" for more details. Values: 
                                        atmos_mezz|wav|wav_list|pcm_s16le|pcm_s24le|pcm_f32le.
  -i [ --input ] arg                    Input file or files, in case of multi-file input.
  --input-manifest arg                  Optional input loudness manifest file.
  -o [ --output ] arg                   Output bitstream file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample".
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file".
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details.
  --data-rate arg (=auto)               Target data rate in kbps. List of available data rate 
                                        values depends on the selected encoder mode. Use 
                                        "--morehelp data-rate" for more details.
  --loudness-management arg (=measure_and_correct)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_and_correct|measure_only.
  -c [ --output-channel-layout ] arg (=auto)
                                        Audio output channel configuration. Values: 
                                        auto|mono|stereo|5.1.
  -e [ --encoder ] arg                  Encoder configuration. Use "--morehelp encoder" for more 
                                        details.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        cc|add-silence|data-rate|embed-timecode|encoder|examples|ex
                                        tras|input-format|loudness-management|loglevel|all.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


Execution time: 0.0006976 seconds
Exit code: 0
```

## `dee_ddpjoc_encoder.exe --help`

```text
Dolby Digital Plus with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.1.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --cc arg (=1)                         Enable concurrent processing. Use "--morehelp cc" for more 
                                        details. Values: 0|1.
  --input-format arg                    Input format followed by format-specific options. Use 
                                        "--morehelp input-format" for more details. Values: 
                                        atmos_mezz|cbi_wav.
  -i [ --input ] arg                    Input file path. Can be provided multiple times. The order 
                                        of input parameters matters and is used as an encoding 
                                        order.
  -o [ --output ] arg                   Output bitstream file path. Can be provided multiple times.
                                        Output paths are matched to inputs based on their order of 
                                        appearance in command string.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'. Allowed for single 
                                        input only.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample". Allowed for single input only.
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file". Allowed 
                                        for single input only.
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details. In case of multiple outputs, silence will
                                        be prepended to the first file and/or appended to the last 
                                        one.
  --data-rate arg (=auto)               Target data rate in kbps. Values: 
                                        auto|384|448|576|640|768|1024.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only.
  -e [ --encoder ] arg                  Encoder configuration. Use "--morehelp encoder" for more 
                                        details.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        cc|add-silence|encoder|examples|extras|input-format|loudnes
                                        s-management|loglevel|all.


Execution time: 0.0007114 seconds
Exit code: 0
```

## `dee_ddpjoc_encoder.exe --morehelp input-format`

```text
Argument of option '--input-format' can be followed by the list of format-specific options.

Single interleaved WAVE file input with channel-based immersive content:
  Example: --input-format cbi_wav:height_trim_5_1=-6:surround_trim_5_1=-9 --input file.wav
  Options:
   height_trim_5_1=<STRING>      (default: auto). Custom 5.1 height trim level. Values: auto|-3|-6|-9|-12.
   surround_trim_5_1=<STRING>    (default: auto). Custom 5.1 surround trim level. Values: auto|0|-3|-6|-9.

Single Dolby Atmos file (DAMF, BWF ADM or MXF IAB):
  Example: --input-format atmos_mezz --input file.atmos
  Options: NONE



Execution time: 0.0005803 seconds
Exit code: 0
```

## `dee_ddpjoc_encoder.exe --morehelp examples`

```text
Examples:
Specifying Dolby Atmos ADM BWF as input
  dee_ddpjoc_encoder --input-format atmos_mezz --input /path/to/input.wav --output /path/to/output.ec3

Specifying MXF IAB as input
  dee_ddpjoc_encoder --input-format atmos_mezz --input /path/to/input.mxf --output /path/to/output.ec3

Specifying Dolby Atmos master file set as input
  dee_ddpjoc_encoder --input-format atmos_mezz --input /path/to/input.atmos --output /path/to/output.ec3

Specifying WAV channel-based immersive 9.1.6 file as input
  dee_ddpjoc_encoder --input-format cbi_wav --input /path/to/input_9_1_6.wav --output /path/to/output.ec3

Specifying multiple input files
  dee_ddpjoc_encoder --input-format atmos_mezz --input /path/to/input1.wav --input /path/to/input2.wav --input /path/to/input3.wav--output /path/to/output1.ec3 --output /path/to/output2.ec3 --output /path/to/output3.ec3

Prepending 5 seconds of silence to the first output file and appending 10 seconds to the last one
  dee_ddpjoc_encoder --add-silence 5:10 --input-format atmos_mezz --input /path/to/input1.wav --input /path/to/input2.wav --output /path/to/output1.ec3 --output /path/to/output2.ec3

Trimming file using timecode with seconds and framerate
  dee_ddpjoc_encoder --start 01:00:04:13 --end 01:43:00:05 --timecode-frame-rate 24 --input-format atmos_mezz --input /path/to/input.wav --output /path/to/output.ec3

Trimming file using timecode with frames
  dee_ddpjoc_encoder --start 01:00:04.2 --end 01:43:00.14 --timecode-frame-rate 24 --input-format atmos_mezz --input /path/to/input.wav --output /path/to/output.ec3

Setting ltrt downmix mode, -1.5 dB center mix, and -6 dB surround mix
  dee_ddpjoc_encoder --encoder preferred_downmix_mode=ltrt:ltrt_cmix=-1.5:ltrt_smix=-6 --input-format atmos_mezz --input /path/to/input.wav --output /path/to/output.ec3

Configuring loudness management options
  dee_ddpjoc_encoder --loudness-management measure_only:preset=manual:dialogue_intelligence=1:speech_threshold=15 --input-format atmos_mezz --input /path/to/input.wav --output /path/to/output.ec3

Setting height trim to -6 in CBI input
  dee_ddpjoc_encoder --input-format cbi_wav:height_trim_5_1=-6 --input /path/to/input.wav --output /path/to/output.ec3

Multiplexing output to MP4
  dee_ddpjoc_encoder --input-format atmos_mezz --input /path/to/input.atmos --output /path/to/output.ec3
  mp4muxer --input-file /path/to/output.ec3 --input-format ec3 --mpeg4-comp-brand dby1 --output-format mp4 --output-file /path/to/output.mp4



Execution time: 0.0005825 seconds
Exit code: 0
```

## `atmos_info.exe --help`

```text
Atmos Info
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 2.0.0 (Oct 17 2024):
  -h [ --help ]             Show this help.
  --loglevel arg (=info)    Logging level followed by log customization options. Use "--morehelp 
                            loglevel" for more details. Values: debug|error|info|quiet|warning.
  -i [ --input ] arg        Input Dolby Atmos mezzanine file.
  --validate arg (=1)       Validate ADM BWF input.
  -v [ --version ] arg (=0) Show version information.


Execution time: 0.0004197 seconds
Exit code: 0
```

## `drp.exe --help`

```text
Dolby Reference Player 3.2.0.8040

Usage:
  drp [options] file

Arguments:
  file                                   File to be played

Options:
  -?, -h, --help                         Display the help text
  -v, --version                          Display version information
  --list-devices                         List available audio devices
  --verbose                              Enable debug prints
  --print-info                           Print bitstream info
  --device <value>                       Select output device from a devices list
                                           Possible values are:
                                           (the audio devices of the machine)
  --audio-out-file <value>               Specify the audio output file path to save raw audio samples as the WAVE format
  --video-out-file <value>               Specify the video output file path to save raw video samples as the YUV420 format
  --metadata-directory <value>           Dump metadata to CSV files
  --volume <value>                       Set volume, 1.0 = 100%
                                           Default: 1
                                           Range: 0 - 10
  --out-ch-config <value>                Set output channel configuration
                                           Default: 2.0
                                           Possible values are:
                                           2.0             channels: L,R
                                           3.1             channels: L,R,C,LFE
                                           5.1             channels: L,R,C,LFE,Ls,Rs
                                           7.1             channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs
                                           9.1             channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw
                                           5.1.2           channels: L,R,C,LFE,Ls,Rs,Ltm,Rtm
                                           5.1.4           channels: L,R,C,LFE,Ls,Rs,Ltf,Rtf,Ltr,Rtr
                                           7.1.2           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltm,Rtm
                                           7.1.4           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltf,Rtf,Ltr,Rtr
                                           7.1.6           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltf,Rtf,Ltm,Rtm,Ltr,Rtr
                                           9.1.2           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltm,Rtm
                                           9.1.4           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltf,Rtf,Ltr,Rtr
                                           9.1.6           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltf,Rtf,Ltm,Rtm,Ltr,Rtr
  --channel-map <value>                  Set mapping of decoded channels to output channels. The map is an array of pairs where
                                         first element in pair is a numerical value of channel id from GstAudioChannelPosition,
                                         and second element is an index of an output channel. For example, to switch stereo
                                         channels, set channel-map to "<<0,1>,<1,0>>"
  --ac3dec-dmx-mode <value>              Set two-channel downmix mode
                                           Default: auto
                                           Possible values are:
                                           auto            Auto detect
                                           lt/rt           Surround compatible
                                           lo/ro           Stereo
  --ac3dec-drc-boost <value>             Set the dynamic range control boost scale factor
                                           Default: 100
                                           Range: 0 - 100
  --ac3dec-drc-cut <value>               Set the dynamic range control cut scale factor
                                           Default: 100
                                           Range: 0 - 100
  --ac3dec-drc-mode <value>              Set the dynamic range control mode
                                           Default: line
                                           Possible values are:
                                           custom-0        Custom mode, analog dialnorm
                                           custom-1        Custom mode, digital dialnorm
                                           line            Line out mode
                                           rf              RF mode
                                           portable-8      Portable mode -8dB (output reference level is -8dB)
                                           portable-11     Portable mode -11dB (output reference level is -11dB)
                                           portable-14     Portable mode -14dB (output reference level is -14dB)
  --ac3dec-drc-suppress <value>          Suppress dynamic range control
                                           Default: false
                                           Possible values are: true, false
  --ac3dec-drop-delay <value>            Drop delay samples added by the decoder at the stream start
                                           Default: false
                                           Possible values are: true, false
  --ac4dec-ajoc-core-enabled <value>     Enable advanced joint object coding core decoder
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-dap-enabled <value>           Enable Dolby Audio Processing
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-de-level <value>              Set Dialog Enhancement preferred Level
                                           Default: 0
                                           Range: -12 - 12
  --ac4dec-dmx-mode <value>              Set stereo downmix mode
                                           Default: lo/ro
                                           Possible values are:
                                           lo/ro           Stereo
                                           lt/rt           Surround compatible, Pro Logic
                                           plII            Surround compatible, Pro Logic II
                                           hp              Headphone virtualization
                                           sp              Speaker virtualization
  --ac4dec-drc-enabled <value>           Enable dynamic range control
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-drop-delay <value>            Drop delay samples added by the decoder at the stream start
                                           Default: false
                                           Possible values are: true, false
  --ac4dec-front-speaker-angle <value>   Provide a front speaker angle
                                           Default: 10
                                           Range: 0 - 30
  --ac4dec-ieq-profile <value>           Select Intelligent Equalizer profile
                                           Default: disabled
                                           Possible values are:
                                           disabled        Disabled
                                           detailed        Detailed
                                           balanced        Balanced
                                           warm            Warm
  --ac4dec-ieq-strength <value>          Select Intelligent Equalizer strength
                                           Default: 10
                                           Range: 0 - 16
  --ac4dec-limiter-enabled <value>       Enable limiter
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-lpde-enabled <value>          Enable loudness-preserving mode in Dialogue Enancement
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-main-assoc-mix-level <value>  Set main and associated mixing level
                                           Default: 0
                                           Range: -32 - 32
  --ac4dec-main-assoc-mode <value>       Set main and associated decoding modes
                                           Default: all
                                           Possible values are:
                                           all             All
                                           main            Main
                                           associated      Associated
  --ac4dec-out-cplx-level <value>        Set output complexity level
                                           Default: 5.1.2
                                           Possible values are:
                                           2.0             2.0
                                           5.1             5.1
                                           5.1.2           5.1.2
                                           5.1.4           5.1.4
  --ac4dec-out-ref-level <value>         Set output reference level. Values from -6 to -1 are not permitted!
                                           Default: -31
                                           Range: -31 - 0
  --ac4dec-pres-index <value>            Set presentation index
                                           Default: 0
                                           Range: 0 - 512
  --truehddec-drc-boost <value>          Set the dynamic range control boost scale factor
                                           Default: 100
                                           Range: 0 - 100
  --truehddec-drc-cut <value>            Set the dynamic range control cut scale factor
                                           Default: 100
                                           Range: 0 - 100
  --truehddec-drc-mode <value>           Set the dynamic range control mode
                                           Default: disabled
                                           Possible values are:
                                           disabled        DRC disabled
                                           follow          Normal mode follows bitstream control flag
                                           normal          Normal mode
                                           heavy           Heavy mode
  --truehddec-presentation <value>       Set presentation
                                           Default: 0
                                           Possible values are:
                                           0               Auto presentation
                                           2               2-Channel presentation
                                           6               6-Channel presentation
                                           8               8-Channel presentation
                                           16              16-Channel presentation
  --oar-bass-ext-mode <value>            Enable bass extraction with the given cutoff frequency
                                           Default: off
                                           Possible values are:
                                           off             Bass extraction disabled
                                           45              Bass extraction enabled at 45Hz
                                           50              Bass extraction enabled at 50Hz
                                           55              Bass extraction enabled at 55Hz
                                           60              Bass extraction ena
```

## `drp.exe --version`

```text
Dolby Reference Player 3.2.0.8040
```

<!-- hand-written below -->

## DEE 5.2.1 `ddp_decode` as a reference decoder

Template `dee\xml_templates\ddp_decode\ec3_ddp_decode_wav_manifest.xml` (`ac3_...` for AC-3). Input
`<ec3><stream_type>non_atmos</stream_type>` decodes the 5.1 core of a JOC
stream; `<downmix_config>off`, `<drc>none` give the coded channels without
dynamic range control or dialogue normalisation (gain 1.0000 measured
against FFmpeg with `-drc_scale 0`). Output is 24-bit WAVE (RF64 beyond
4 GiB) in WAVE order, and it starts 256 samples later than FFmpeg or oadec:
the decoder drops the first half block. The manifest output needs a file
name and the temp directory must exist. The template placeholders
`FILE_NAME`/`PATH` appear once per input, output and temp entry; replace
them in order. A 105-second clip decodes in 5 s, a two-hour film in about
six minutes.

It decorrelates the coupled channels of an E-AC-3 stream, as the element of
the Reference Player does and to the same samples: the two stand at 110 to
129 dB from each other in the coupled region of four streams, which is the
24 bits this one writes. An AC-3 stream neither touches. `docs/eac3.md` has
what they do and how it was read.

## Enhanced coupling is not an encoder option

None of the captured command lines mentions coupling at all, and the
`pcm_to_ddp` and `encode_to_atmos_ddp` job schemas expose only the encoder
mode and the data rate. No Dolby tool here can be asked to emit enhanced
coupling, which is why `oadec eac3-ecpl-inject` exists.

An encoder outside this set writes it unasked: MainConcept TotalCode Studio
5.3, whose Dolby Digital Plus Professional encoder couples a stereo
programme from the lowest sub-band up to 80 kbit/s, and uses standard
coupling from 96 kbit/s on and in 5.1 at every rate tried. Its streams are
what showed the two things the bit allocation does there (`docs/eac3.md`).
It is driven from its command line with a preset file,
`tcstudio <preset.mps> <preset name> <input> <output> -transcode -quit`; it
names its output after the input whatever it is told, and takes a preset
file changed by hand, saying so in its log.

Both Dolby decoders here also ignore the enhanced coupling angle and chaos
fields: a stream with zeros in them and a stream with a full spread decode
bit-identically, in the `ddp_decode` filter and in the Reference Player. See
`docs/eac3.md`.

## The Reference Player also decodes to objects

`drp.exe` refuses `--out-ch-config` beyond 7.1 when writing a file, but the
player ships Dolby's GStreamer elements next to it and they have no such
limit. `gst-plugins/` holds `dlbac3parse`, `dlbac3dec`, `dlbtruehdparse`,
`dlbtruehddec`, `dlbac4parse`, `dlbac4dec`, `dlboar` (Object Audio Renderer,
2.0 up to 9.1.6) and `dlbaudiodecbin`, which wires a decoder to the renderer.

Set the environment first: `GST_PLUGIN_PATH` to the `gst-plugins` directory
and the player's own directory on `PATH`.

**Rendered bed**, any layout the renderer supports:

```
gst-launch-1.0 filesrc location=in.ec3 ! dlbac3parse enable-metadata=true \
  ! dlbaudiodecbin out-ch-config=7.1.4 ! audioconvert ! wavenc \
  ! filesink location=out.wav
```

**The objects themselves.** `dlbaudiodecbin` sets the decoder to
`out-ch-config=RAW` (21) when the content is Atmos, and that value can be
given directly. It is not in the element's own enumeration, which stops at
7.1, but the property takes it:

```
gst-launch-1.0 filesrc location=in.ec3 ! dlbac3parse enable-metadata=true \
  ! dlbac3dec out-ch-config=21 drc-suppress=true drc-mode=custom-0 \
              drc-cut=0 drc-boost=0 drop-delay=true \
  ! "audio/x-raw(meta:DlbObjectAudioMeta),format=F32LE" \
  ! identity ! filesink location=objects.f32
```

The result is headerless 32-bit float, one channel per element of the OAMD
programme: for a 5.1 JOC stream with 15 objects that is the LFE bed and then
the 15 objects, in the order the programme lists them. The element writes
sixteen channels whatever the programme has: a stream of an LFE and eleven
objects leaves the last four silent (7 such streams of the matrix of round
trips below, and 44 with fifteen). `drc-suppress` and
`drc-mode=custom-0` keep dynamic range control and dialogue normalisation
out; `drop-delay=true` removes the decoder's start-up samples, after which
the objects line up with `oadec decode --format damf --no-bed-conform` at
lag zero.

**The channels the same way.** With `out-ch-config=0`, `7` or `11` in place
of `21` and `audio/x-raw,format=F32LE` for the caps, the same element writes
the coded channels of a stream of two, six or eight as 32-bit floats in WAVE
order, with no dialogue normalisation and no dynamic range control. Three
things about that decode, each measured:

- it leaves out the first block: its first sample is sample 256 of `oadec
  decode`;
- it is the same bytes every time, the dither included, which is how the
  dither could be read off it (`docs/eac3.md`, `tools/dither_probe.py`);
- it keeps a sample at full scale: where a stream decodes to 1,0054 it
  writes 1,0.

**TrueHD needs a container.** `dlbtruehddec` has the same `out-ch-config=21`
and a `presentation` property that reaches 16, but it will not take a raw
`.thd` elementary stream: it answers `0x50` on the first access unit whether
the parser is in passthrough or not, at any presentation. Wrap the same audio
in MP4 and it decodes. `qtdemux` here does not know the `mlpa` sample entry
and labels the samples `audio/x-gst-fourcc-mlpa`, so the caps need fixing on
the way past:

```
ffmpeg -i in.thd -c:a copy -strict -2 -movflags faststart -f mp4 in.mp4

gst-launch-1.0 filesrc location=in.mp4 ! qtdemux   ! capssetter caps="audio/x-true-hd" replace=true join=false   ! dlbtruehdparse enable-metadata=true   ! dlbtruehddec out-ch-config=21 presentation=16   ! "audio/x-raw(meta:DlbObjectAudioMeta),format=S32LE"   ! identity ! filesink location=objects.s32
```

The result is 32-bit words, one channel per element of the programme, padded
to sixteen with digital silence. Our object output matches it sample for
sample on a presentation whose dialogue normalisation is −31 dB, which Pi and
Talk to Me carry; see `docs/evidence/2026-09-10.md`.

**The object mode applies dialogue normalisation, with dither.** On a
presentation that signals any other dialogue norm, Dolby's words are the coded
samples times the dialnorm gain plus triangular dither of ±1 LSB at 24 bits,
kept in full 32-bit precision; `drc-mode` has no setting that turns it off.
Measured on five UHD titles with dialogue norm −27 and −28 dB: the fitted gain
is the nominal one within 3·10⁻⁵ dB, the residual after it has a standard
deviation of 0.408 LSB (1/√6, the triangular distribution exactly), is
uncorrelated with the signal, and is present where the coded samples are
digital silence. Ten titles at −31 dB are bit-exact; `truehdd` and FFmpeg
agree with oadec on all sixteen. So a byte comparison against this element is
only meaningful at −31 dB, and a title that fails it elsewhere has to be
judged against `truehdd`. See
`docs/audit/evidence/verification-2026-09-14/06-truehd-dolby-dialnorm.json`.

This is the only Dolby decoder here that works in the object domain, and it
is what settled the JOC matrix alignment and the low-band quadrature filter;
see `docs/evidence/2026-09-09-c.md`.

**Cut from the head, never from the middle.** A twenty-second cut taken ten
minutes into a raw `.thd` is refused with *the same message a genuine refusal
gives*, because a byte seek lands inside an access unit and the preroll never
finds the presentation. Pi from the head is accepted and Pi from 600 seconds is
not. A sweep that seeks reports every title as refused and looks like a
discovery.

**The object mode opens about half of what it is given.** Over 194 object
presentations in 186 library files, `out-ch-config=21` with `presentation=16`
opens 105 and refuses 89, and no field `oadec info --json` reports — nor any
pair of them, of 3 486 tried — separates the two sets. The verdict is stable
across runs and cut lengths and is specific to the object mode: the same clip
decodes at `presentation=16` with the default channel configuration and at
`presentation=2` with `out-ch-config=21`. See
`docs/audit/evidence/remediation/truehd-object-sweep.json`.

**`gst-launch` treats a backslash as an escape** inside a property value, so
every path handed to `filesrc` or `filesink` has to be written with forward
slashes. A Windows path fails at `gst_base_src_start` with "Failed to start",
which reads like a missing file rather than a quoting problem.

## Two things DEE will not do the obvious way

**`pcm_to_ddp` with `encoder_mode` ddp71 refuses loudness correction**:
"Correction is supported only by encoder_mode=ddp". Use `measure_only`, which
is better for a level-sensitive comparison anyway because it leaves the samples
alone. That block also rejects `metering_mode` 1770-4, which the Atmos filters
accept; 1770-3 works.

**DEE ignores the WAVE channel mask.** An eight-channel file is read as
L, R, C, LFE, Ls, Rs, Lrs, Rrs — side pair fifth and sixth, back pair seventh
and eighth — where mask 0x63F orders them L, R, C, LFE, BL, BR, SL, SR.
Encoding the same audio again with the mask set to zero places the tones
identically, so it is not reading the mask at all. It silently swaps the side
and back pairs of anything encoded from a correctly tagged file.

**One knob moves `2ch_control_enabled`.** In `encode_to_dthd`, setting
`presentation_2ch/drc_default_on` to false makes the encoder write the flag
clear; `dolby_surround_encoded`, `dolby_headphone_encoded`,
`legacy_authoring_compatibility` false and the speech DRC profile all leave it
set. That is the only way found to author a stream with the flag clear, and it
is what showed the flag is not why the object path refuses anything.

## What the converters write for an object's gain

The Dolby Atmos Conversion Tool 2.1.2 and DEE 5.2.1 `convert_atmos_mezz` both write an
active object's gain into the ADM as ten decimals of the float32 linear factor:
`0.5011872053` for -6 dB, `1.4125375748` for +3 dB, `0.2511886358` for -12 dB and
`0.0000000000` for a muted object; nothing at 0 dB. An inactive object carries `0.0` with
`<importance>0</importance>`, which is the profile's marker and a different thing. Float64
arithmetic would print `0.5011872336`, which is how a reader can tell which arithmetic
produced a file. Measured on the audit's C02 scene
(`docs/audit/evidence/adm-remediation/adm-remediation-dolby.json`); the Conversion Tool reads
the four gains back into DAMF as -6, +3, -inf and -12 dB from its own file and from oadec's
alike. The profile text (table 11) reserves `gain` for inactive objects; the converters do not
follow it there, and `atmos_info` 5.7.2 `--validate 1` accepts the result.

Two `convert_atmos_mezz` habits worth knowing. The output `file_name` is taken literally:
`c02-dee.wav` gives a file, `c02-dee` gives a directory `c02-dee/output.wav`. And the filter
looks for an `.atmos.dbmd` sidecar beside a DAMF set and warns when there is none
(`Sys - AtmosMasterFileStorageDesc - Error 2 while reading "...atmos.dbmd"`); the conversion
completes without it.

## What the encoders read from a DAMF set

The Conversion Tool writes `warpMode`, `ffoa` and, per object, `decorr` into its own
sets, and the tools accept more keys than that. Whether an encoder listens is a separate
question, so each key was added to an authored set and the set encoded with
`dee_ddpjoc_encoder` and `dee_dthd_encoder` (`tools/damf_key_probe.py`). Two
encodes of the unchanged set are bit-identical, so any changed byte is the key's
doing. Evidence: `docs/audit/evidence/metadata-bridge-2026-11-09/02-damf-key-probe.json`.

| Key in the `.atmos` header | Encoder output |
|---|---|
| `warpMode` | **changes**: see below |
| `dialNorm`, `ffoa`, `scNumberOfElements` | identical |
| `downmixType_5to2`, `downmixType_7to5`, `bedDistribution`, `positionSmoothed`, `51-to-20_LsRs90degPhaseShift`, `surroundTrim_5_1`, `surroundTrim_7_1` | identical (`downmixType_*` take `LoRo_Stereo`, `LtRt_ProLogic`, `LtRt_PLII` and `LoRoStandard`, `ProLogicIIx`, `directRender`; the others are boolean; a wrong value refuses the file) |
| `trimMode` and the trim keys | identical in every layout tried, and `atmos_info` still reports auto trims |

Dialogue normalisation comes from the encoder's own measurement unless it is
given on the command line: `dee_dthd_encoder --loudness-management
skip:custom_dialnorm=-20` writes -20 for the 6- and 8-channel presentations and -26 for
the 2-channel one. `--start 0` is not needed for a set whose offset is zero.

**A `.atmos.dbmd` beside the set changes the warp mode.** Any file at that path,
even an empty one, turns a set with no `warpMode` into Lo/Ro: the encode is
bit-identical to `warpMode: LoRo`. `warp_mode` and the trim fields inside it are
refused with "Not user-modifiable DBMD values detected", so it is no route to
either, and `warpMode` in the header overrides it. oadec writes no such file.

## The warp mode

| `warpMode` in `.atmos` | Stream `warp_mode` | `dbmd` segment 9, byte 152 | Conversion Tool `--set_warp_mode` |
|---|---:|---:|---|
| `normal` | 0 | `0x80` | `normal` |
| `warping` | 1 | `0x81` | `warping` |
| `ProLogicIIx` | 2 | `0x82` | `downmix_pliix` |
| `LoRo` | 3 | `0x83` | `downmix_loro` |
| *(none)* | **1** | `0x84`, "not indicated" | |

TS 103 420 clause 5.6.5.1 defines 0 as no position adjustment and 1 as the Y
position multiplied by two, and reserves 2 and 3; the encoder writes them anyway and
24 tracks carry 3. An output that says nothing is encoded as warping, while the
library's E-AC-3 streams declare normal 78 times, warping 28 and Lo/Ro 24, so the
default trip changes the mode of 102 of 130. `oadec decode --warp-mode stream` keeps it, through
either output: on five titles the mode survives with the flag, becomes warping
without it, and the DAMF and ADM routes encode to the same bytes.

It matters unevenly. Rendered by the player's own renderer, on the same objects
(`04-warp-render.json`):

- normal against warping changes the render: on two film heads at 5.1, the surround
  pair 1 to 3 dB down and the front pair up to 1 dB up, with the difference 10 to
  16 dB below the signal in the front pair; at 2.0 about the same.
- Lo/Ro against warping is below any level that matters: −58 dB and lower, on every
  channel of two heads.
- warping against warping is bit-identical, the control.

A TrueHD stream carries no trim element, so it has no mode to keep. The mode still
shapes what its encoder derives: `warpMode: normal` changes the 8-, 6- and 2-channel
presentations and the measured dialogue normalisation. For a title that also has an
E-AC-3 Atmos track, that track says which mode the mix was delivered with:
`oadec oamd --json` reports it as `warp_modes`.

## The DRC profile is the other thing a second encode changes

`oadec emdf` reads the programme information payload (TS 102 366 clause H.3.2), which
is the only place a stream says which DRC profile its gain words were made with. The
encoders default to `film_light`, and the library's 133 E-AC-3 Atmos tracks carry
`film_light` 71 times, `none` 36, `music_light` 25 and `film_standard` once, so the
default trip changes 62 of them. It is not in a DAMF set; it goes on the command line:

```text
oadec emdf source.ec3            # Programme: ... DRC profile line {"none": ...}, RF {"none": ...}
dee_ddpjoc_encoder ... -e drc_profile_line=none:drc_profile_rf=none
```

`dynrngprof` is the line mode profile and `comprprof` the RF one: six pairs asked of
the encoder come back as asked. It matters less than the warp mode and it does matter.
An hour into 1917, whose profile is `none`, the default encode plays in line mode
0.45 dB quieter over a minute and up to 1.47 dB in a 100 ms window; a title already at
`film_light`, the control, differs by nothing (`12-programme-payloads.json`).

The payloads hold up. Over 691 956 loudness and 261 647 programme information payloads
none is unreadable and none is the wrong size. Where the correction was dialogue gated,
the speech-gated loudness equals the dialogue normalisation on 106 of 107 tracks, as
clause H.3.1.3.10 says it shall, and the other is below what the field can say (-33.5
LUFS against -31). `ph90filt` is clear on exactly the three tracks at downmix
configuration 0. 229 channel-based tracks carry a loudness practice the table reserves.

## The dialogue normalisation and the downmix are the rest of it

Two more things a stream says of its programme are in no master, and an encoder
that is told nothing writes its own.

**The dialogue normalisation.** An encoder measures the loudness of what it is
given and writes the dialogue normalisation it finds, so a second generation says
another than the first wherever the measurement of the decode comes out otherwise:
19 rows of 51 in the matrix below. Both encoders take one by hand:

```text
dee_ddpjoc_encoder ... --loudness-management skip:custom_dialnorm=-27
dee_dthd_encoder   ... --loudness-management skip:custom_dialnorm=-27
```

The help of `dee_ddpjoc_encoder` names `measure_only` and nothing else. It takes
`skip` all the same, and refuses it without `custom_dialnorm`. Told -31, -24, -11
and -1, each encoder writes what it is told: the E-AC-3 stream says it as its
`dialnorm`, the TrueHD stream as the dialogue normalisation of its 6-, 8- and
16-channel presentations, with the 2-channel one 6 dB under it. There is a price.
With `skip` the encoder measures nothing: the loudness payloads of the stream give
the figure it was told as the gated loudness, and no short-term loudness and no
true peak, where a stream of the same master that was measured has 70 of the one
and 17 of the other.

**The downmix.** A stream of more than two channels can say which downmix to two
it prefers and four levels for it (`docs/eac3.md`, *What a stream says of its
downmix*). The encoders default to Lo/Ro and four times -3 dB, and take all five
with `--encoder`:

```text
dee_ddpjoc_encoder ... --encoder preferred_downmix_mode=ltrt:loro_cmix=-1.5:loro_smix=-6:ltrt_cmix=-4.5:ltrt_smix=-1.5
```

`ltrt-pl2` is written as the code the table of the standard reserves, and the
AC-3 encoder refuses it. The TrueHD encoder has no such option, and a TrueHD
stream says no downmix of this kind.

`tools/encoder_arguments.py` reads a stream and writes all of it out, the DRC
profiles of the section above with it, and says what it leaves to the encoder: a
field of which the stream says two things, as a splice does, and a field of
which it says nothing.

```text
$ python tools/encoder_arguments.py first.ec3
E-AC-3 (dee_ddpjoc_encoder): --loudness-management skip:custom_dialnorm=-27 --encoder preferred_downmix_mode=ltrt:loro_cmix=-1.5:loro_smix=-6:ltrt_cmix=-4.5:ltrt_smix=-1.5:drc_profile_line=music_standard:drc_profile_rf=speech
TrueHD (dee_dthd_encoder):   --loudness-management skip:custom_dialnorm=-27
the decode (oadec decode):   --warp-mode stream
```

Whether it comes back was measured (`tools/interop.py programme`): four scenes,
each encoded three times with a programme that is not the encoder's defaults,
decoded to a master with `--warp-mode stream`, and the master encoded twice.

| What the first stream says | Second generation, encoder told nothing | Told the arguments |
|---|---:|---:|
| the dialogue normalisation | 3 of 12 | 12 of 12 |
| the downmix it prefers | 0 of 12 | 12 of 12 |
| the four levels of the downmix | 0 of 12 | 12 of 12 |
| the DRC profile of line mode | 0 of 12 | 12 of 12 |
| the DRC profile of RF mode | 0 of 12 | 12 of 12 |
| the warp mode (which the master has) | 12 of 12 | 12 of 12 |

Every one of the 36 streams is clean. None of the twelve that were told the
arguments has a measured true peak, and all twelve that were told nothing have:
whoever wants the measurement gives the encoder the `--encoder` part alone.

Four whole tracks of streaming services, E-AC-3 with JOC at 768 kbit/s and 1.7 to
2.6 hours long, were read the same way. Each says one dialogue normalisation from
its first frame to its last (-25 to -29), `film_light` for both DRC profiles and
the Lo/Ro warp mode, and none says a downmix: their frames carry no mixing
metadata, and the tool leaves the downmix to the encoder and says so
(`audit/evidence/engine-round-2026-11/60-what-a-second-generation-says-of-the-programme.json`).

## `--order stream` is the order the encoders read

Eight tones, one per channel, written in the order `oadec decode --order stream`
produces, encoded with `dee_dthd_encoder` 5.7.2 as an 8-channel presentation and
decoded again with `--order stream`, come back in the slots they went in, with the
channel mask 0x63F and with 0 alike. The default `--order interchange` swaps the
side and back pairs against what the encoder reads. So a WAVE written for one of
these encoders takes `--order stream`, and no conversion
(`05-channel-order.json`).

## `--encoder bluray` writes downmix configuration 4

`dee_ddpjoc_encoder --encoder bluray --data-rate 1152..1664` writes an AC-3 core
and an E-AC-3 dependent substream, 5.1.2 and fifteen objects, with `dmx_config` 4.
That configuration had only been seen in three films; it can now be authored.
On the controlled scene every one of eight objects is recognised and all seven
static positions come back exact, the same as the online mode at configuration 3,
and against the Dolby object decoder the worst object is 52.34 dB and the median
94.08 dB, against 51.80 and 95.47 (`07-config-4-authored.json`). It gave no two
data points and no EMDF in the auxiliary data. Configurations 1 and 2 remain unseen.

## What the encoders do with what a master says of an object

A state of an object in a master says more than a place: a gain, a size, a lock to
the nearest channel (`snap`) and a zone to keep to. No stream carries any of the
four. Over the 269 streams of the matrix below and the 66 of these probes
`oadec oamd` counts no object gain, no size, no lock and no zone but "all", as the
census of a library found no gain, size or lock (`docs/oamd.md`). What the encoders
do with them was asked of one object alone over a silent LFE, with one thing said
of it, at 768 kbit/s and as TrueHD (`tools/interop.py probe`, 33 probes through
each encoder). The two encoders do the same to every probe: the element with most
of the object is at the same place in all 33 and has the same share of it to
0,02. The figures are the E-AC-3 ones, and a distance is the largest difference
in a coordinate of the room.

| The master says | What comes back |
|---|---|
| a place and nothing else, at six places | one element at that place, 0,05 dB from the tone at most |
| `snap: true` at the six | the two that are 0,10 and 0,39 from a speaker of a 7.1 layout come back at the speaker; the four that are 0,42 or more from every one come back where they were put. One element either way |
| `zones`, five values at three places | the object is where the zone lets it be: `screen only` on the front wall at the same x, in one element; the other four in two to four elements at speakers' places, the one with most of it 0,42 to 0,97 from where the object was put. The power over the elements stays within 0,19 dB of the tone |
| `size` 0.1, 0.4, 1.0 | 4, 9 and 11 elements, of which the one with most has 52, 37 and 21 per cent of the power. The power over them is 3,2, 0,8 and 1,1 dB under the tone and their sum 2,4, 7,7 and 8,7 dB over it: they carry it in phase |
| `gain` -6, -20, +6 dB | one element at the place and at the level of the tone, -0,04 dB each time: the gain is neither applied nor carried |

So a lock, a zone and a size are rendered by the encoder into places and elements,
and a gain is dropped, which the audit found of the gain and the size on one scene
(`docs/audit/conformance-matrix.md`, *Controlled object gain* and *size*). It is
the encoders' doing and not the master's: the Conversion Tool carries the same
-6 dB into ADM as `0.5011872053` (above), and the encoder makes the same bytes of
that ADM as of the master. `oadec` reads what a stream says, so none of the four
comes back from a decode, and a master made from a stream has none to lose in a
second encode. Evidence: `docs/audit/evidence/engine-round-2026-11/54-scenes-through-the-encoders-and-back.json`.

## A matrix of round trips

`tools/interop.py` runs scenes through these tools and back and holds what comes
back to the scene (`docs/testing-tools.md`). A row is one value of each of seven
things a programme and its encode can be:

| Axis | Values |
|---|---|
| the frame rate the master names | 23.976, 24, 25, 29.97, 30 |
| the bed | the LFE alone, 2.0, 5.1, 7.1, 7.1.2, 5.0 |
| objects | 1, 6, 15, 20 |
| how the first three move | not at all; twice; there and back; round a circle, a new place every 2048 samples |
| what the first object says besides its place | nothing, a gain, a size, a lock, a zone; or the master names the Lo/Ro warp mode |
| how the elements sound | together, or in turn, each alone |
| the encoder | 384, 448, 576, 640, 768, 1024 kbit/s; `--encoder bluray` at 1536 |

51 rows have every pair of values in them, of 40 320 combinations. Every object and
every channel of a bed has a tone of its own under 3,2 kHz, so it is known by its
sound whatever the encoder numbers it. Each row went five ways: to E-AC-3 JOC; to
the same encoder through the Conversion Tool's ADM; as a second generation, the
first stream decoded by `oadec` to a master and to ADM and encoded again; and to
TrueHD, where the encoder axis picks 12, 14 or 16 elements. That is 269 streams,
218 of them E-AC-3, with the second generations of the Lo/Ro rows made twice.

**Every master is taken and every stream is clean.** Both encoders and the
Conversion Tool took all 51 masters, and the encoder took `oadec`'s ADM of all 51
streams (written with `--dolby-origin-tag`; without it the encoder says "Content
was not authored with Dolby tools"). `verify`, `info`, `oamd` and `emdf` exit 0 on
all 269 streams and every decode exits 0. One thing was refused, by both encoders,
and it is no row: a master at 96 kHz ("Unsupported sample-rate: 96000").

**What a rate writes.** The bed of the stream is the LFE in every row, whatever
the bed of the master.

| Encoder | Rows | Core | Objects | JOC configuration | Parameter bands | Complexity index | Core tools in every stream |
|---|---:|---|---:|---:|---:|---:|---|
| 384 | 7 | E-AC-3 | 11 | 3 | 12 | 12 | coupling and the hybrid transform |
| 448, 576 | 16 | E-AC-3 | 15 | 3 | 12 | 16 | coupling |
| 640, 768, 1024 | 21 | E-AC-3 | 15 | 3 | 12 | 16 | neither |
| `bluray`, 1536 | 7 | an AC-3 core at 640 kbit/s and a dependent E-AC-3 substream | 15 | 4 | 15 | 16 | neither |

A channel of a bed comes back as an object at its speaker's place, 128 of 128 where
the elements sound in turn, and the LFE as the LFE, 16 of 16. Every stream has
one interpolation branch, smooth with one data point, and a clip gain of 1; none
has two data points or a steep object, and 18 have sparse ones, all of them rows
whose elements sound together. Configurations 1 and 2 stay unseen. As TrueHD the
stream has an LFE and 11, 13 or 15 objects for 12, 14 or 16 elements asked.

**What a stream says of the programme.** Every E-AC-3 stream has a trim element
and no TrueHD one has, as in a library. The warp mode is warping unless the master
names one, and Lo/Ro in the 7 rows whose master names that; a TrueHD stream has
none. In E-AC-3 the sample offset and the block offset factor are 0 in every
payload and every ramp is 1536 samples but one of 512 or 1024 in 6 of the 7 `bluray`
streams; in TrueHD the offsets go round 0, 8, 16 and 24, the factor is 0 or 1, and
49 of 51 streams have one ramp that is not 1536, of 96 to 1344 samples. The dialogue
normalisation is the encoder's own measurement, 9 to 31.

**The two decoders make the same elements.** Dolby's object decode lines up with
`oadec decode --format damf --no-bed-conform` at lag 0 in all 51 rows, and in both
second generations. The element furthest from Dolby's is 54,85 to 110,76 dB
from it (median 65,28), and in 34 of the 51 rows it is one that carries a tone under
375 Hz, the lowest subband, where `docs/joc.md` says what is left; a second
generation stands at 70,13 to 109,89.

**What comes back.** A rest is a stretch an object stays at a place for; the
figures are of the objects a scene says nothing of but a place, and "at the place"
is within 0,05 of it.

| | E-AC-3 JOC | TrueHD |
|---|---|---|
| elements that sound in turn | 296 rests: 296 at the place, 296 in one element, -0,21 to +0,22 dB from the tone (median -0,01); 54 of 54 moves made, 896 to 3776 samples after the master's (median 2720) | 296 rests: 296 at the place, 296 in one element, at the level of the tone to the hundredth of a dB; 54 of 54 moves made, 1152 to 4032 samples after the master's (median 2976) |
| together, in a scene of no more elements than the stream has | 84 rests: 84 at the place, 58 in one element, -3,45 to +0,27 dB from the tone (median -0,19); 27 of 27 moves made, 128 to 3584 samples after the master's (median 3200) | 66 rests: 66 at the place, 66 in one element, at the level of the tone to the hundredth of a dB; 23 of 23 moves made, 384 to 3840 samples after the master's (median 3456) |
| together, in a larger scene | 257 rests: 133 at the place, 44 in one element, -7,28 to +0,37 dB from the tone (median -1,34); 21 of 36 moves made, 1664 to 5120 samples after the master's (median 3200) | 275 rests: 122 at the place, 161 in one element, -4,28 to 0,00 dB from the tone (median -0,35); 16 of 40 moves made, 1920 to 5376 samples after the master's (median 3456) |

What sounds alone comes back whole, however many elements the scene has: the
encoder's spatial coding gives what is sounding an element of its own. Elements
that sound together and fit come back at their places, in TrueHD untouched and in
JOC with a part of a tone in other elements, as a coder that rebuilds the objects
from a downmix band by band does to steady tones that share a band: it has twelve
or fifteen bands and the scenes up to thirty tones. A scene larger than the stream
is put into clusters, and an object is then where its cluster is.

**Between the ways.**

- The same master encoded again is the same E-AC-3 bytes, 116 of 116.
- Through the Conversion Tool the stream is the same bytes as from the master,
  51 of 51.
- The second generation made from `oadec`'s master and the one made from its ADM
  are the same bytes, 51 of 51: the two outputs say one programme.
- A second generation keeps what the first kept where the elements sound in turn:
  310 of 329 rests at the place as in the first, each within 0,23 dB of its
  level there. Where they sound together it is a second coding of a coded scene:
  234 at the place against 238, and -2,46 to +0,60 dB (median -0,25).
- The warp mode is lost by a decode that names none: all 7 Lo/Ro rows come back
  as warping, and all 7 as Lo/Ro with `--warp-mode stream` (above).
- The dialogue normalisation is the encoder's measurement, made anew of the
  decode: a second generation has another than the first in 19 of 51 rows, its
  field 8 less to 7 more, and the same DRC profiles in all 51. The help of this
  encoder names `measure_only` and nothing else for `--loudness-management`, and
  this text said on the strength of that help that what a first stream says could
  not be handed on. It can: the encoder takes `skip:custom_dialnorm=` (*The
  dialogue normalisation and the downmix are the rest of it*, above).
- The frame rate a master names changes no byte of an E-AC-3 stream, and of a
  TrueHD stream no more than encoding the same master again does. That encoder's
  streams of one master differ from run to run, here in 6 to 10 bytes of 501 144: the
  check nibble of an access unit and the bit of its first directory entry that
  the syntax reserves, which these streams set in about 1650 of 3 600 units. They decode
  to the same samples in every presentation and to the same objects, and so do
  the five frame rates, which differ in 2 to 12 such bytes.
- The whole matrix was run twice. Every E-AC-3 stream was the same bytes both
  times, 204 of 204 over the four ways, and no TrueHD stream was, 0 of 51; every
  figure of what came back was the same, for all 255 rows.

## What the object refusals are not

**The TrueHD refusal is a validity check, not a stream defect.** At
`GST_DEBUG=dlbtruehddec:7` the decoder plays the presentation, then its Evolution
validity check fails and it reports "presentation is not available". Everything
measured since fits a check over protected metadata: no field of 84 separates the
two sets, any in-place edit is refused whatever it changes, and the same objects
encoded again open. `truehdd` and FFmpeg decode these streams and agree with oadec.
Why an untouched disc fails is not known, and looking for a separating field is
not the way to find out.

**The E-AC-3 refusal is still open, and it is not the trim element.** Three of 133
tracks carry no trim element, all at configuration 0. Two drop to six channels and
"Dolby Atmos : No" (Snatch and one other, dialogue normalisation 31), and one opens
with sixteen (dialogue normalisation 26), so the element's absence is not the
cause (`08-dolby-no-trim.json`). A stream edited in place shows the same symptom
(`remediation/payload-rewrite-rejected.json`), which is a resemblance and not a
measurement of these two.
