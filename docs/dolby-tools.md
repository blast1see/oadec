# Dolby tool command lines on the development machine

Captured 2026-09-06 by `tools/capture_dolby_help.py`. These outputs
are the ground truth for the flags used by the verification scripts.

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


Time elapsed: 0.50999 seconds
```

## `dee.exe --print-stages`

```text
[2026-09-06 23:36:34.259] INFO: Dolby Encoding Engine, version: 5.2.1-5994839.
[2026-09-06 23:36:34.263] INTERNAL_INFO: Loading library "C:\dee\\dactlib.dll".
[2026-09-06 23:36:34.263] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_cod.dll".
[2026-09-06 23:36:34.263] INTERNAL_INFO: "C:\dee\\dee_audio_filter_cod.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.263] INTERNAL_INFO: "C:\dee\\dee_audio_filter_cod.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.265] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_convert_atmos_mezz.dll".
[2026-09-06 23:36:34.265] INTERNAL_INFO: "C:\dee\\dee_audio_filter_convert_atmos_mezz.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.265] INTERNAL_INFO: "C:\dee\\dee_audio_filter_convert_atmos_mezz.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.267] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_ddp.dll".
[2026-09-06 23:36:34.267] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.267] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.268] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_ddp_atmos.dll".
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_atmos.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_atmos.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.268] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_ddp_single_pass.dll".
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_single_pass.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_single_pass.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.268] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_ddp_transcode.dll".
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_transcode.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.268] INTERNAL_INFO: "C:\dee\\dee_audio_filter_ddp_transcode.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.273] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_dthd.dll".
[2026-09-06 23:36:34.273] INTERNAL_INFO: "C:\dee\\dee_audio_filter_dthd.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.273] INTERNAL_INFO: "C:\dee\\dee_audio_filter_dthd.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.273] INTERNAL_INFO: Loading library "C:\dee\\dee_audio_filter_edit_ddp.dll".
[2026-09-06 23:36:34.273] INTERNAL_INFO: "C:\dee\\dee_audio_filter_edit_ddp.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.273] INTERNAL_INFO: "C:\dee\\dee_audio_filter_edit_ddp.dll" is a component of audio_filter type.
[2026-09-06 23:36:34.275] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_custom_yuv_sink_buffered_writer.dll".
[2026-09-06 23:36:34.275] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_custom_yuv_sink_pipe_cmd.dll".
[2026-09-06 23:36:34.277] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_it_burn_subtitles.dll".
[2026-09-06 23:36:34.317] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_j2k_dec_base.dll".
[2026-09-06 23:36:34.319] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_mp4_mux_base.dll".
[2026-09-06 23:36:34.319] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_noise_base.dll".
[2026-09-06 23:36:34.319] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_scaling_base.dll".
[2026-09-06 23:36:34.319] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_tiff_dec_libtiff.dll".
[2026-09-06 23:36:34.319] INTERNAL_INFO: Loading library "C:\dee\\dee_plugin_ts_mux_base.dll".
[2026-09-06 23:36:34.322] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_42.dll".
[2026-09-06 23:36:34.322] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_42.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.322] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_42.dll" is a component of video_filter type.
[2026-09-06 23:36:34.323] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_5.dll".
[2026-09-06 23:36:34.323] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_5.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.323] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_5.dll" is a component of video_filter type.
[2026-09-06 23:36:34.325] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_7.dll".
[2026-09-06 23:36:34.325] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_7.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.325] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_7.dll" is a component of video_filter type.
[2026-09-06 23:36:34.326] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_81.dll".
[2026-09-06 23:36:34.326] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_81.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.326] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_81.dll" is a component of video_filter type.
[2026-09-06 23:36:34.327] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_82.dll".
[2026-09-06 23:36:34.327] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_82.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.327] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_82.dll" is a component of video_filter type.
[2026-09-06 23:36:34.327] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_hdr10.dll".
[2026-09-06 23:36:34.327] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_hdr10.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.327] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_hdr10.dll" is a component of video_filter type.
[2026-09-06 23:36:34.329] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_dv_sdr.dll".
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_sdr.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_dv_sdr.dll" is a component of video_filter type.
[2026-09-06 23:36:34.329] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_extract_prores.dll".
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_extract_prores.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_extract_prores.dll" is a component of video_filter type.
[2026-09-06 23:36:34.329] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_hevc_encode.dll".
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_hevc_encode.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.329] INTERNAL_INFO: "C:\dee\\dee_video_filter_hevc_encode.dll" is a component of video_filter type.
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_hevc_transcode.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: "C:\dee\\dee_video_filter_hevc_transcode.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.330] INTERNAL_INFO: "C:\dee\\dee_video_filter_hevc_transcode.dll" is a component of video_filter type.
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\dee_video_filter_parse_mxf.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: "C:\dee\\dee_video_filter_parse_mxf.dll" is a DEE's component: Version=5.2.1.
[2026-09-06 23:36:34.330] INTERNAL_INFO: "C:\dee\\dee_video_filter_parse_mxf.dll" is a component of video_filter type.
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libelprocessor_dll.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libmetadata_postproc_dll.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libmezzanine.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libnbc_preproc_dll.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libpreproc.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\librpu.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libslbc_dll.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libslidm_dll.dll".
[2026-09-06 23:36:34.330] INTERNAL_INFO: Loading library "C:\dee\\libvesmux.dll".
[2026-09-06 23:36:34.333] INTERNAL_INFO: Loading library "C:\dee\\Mediainfo.dll".
[2026-09-06 23:36:34.333] INTERNAL_INFO: Loading library "C:\dee\\tiff.dll".
[2026-09-06 23:36:35.350] INFO: CPU: 4.3 % 7.6 %, MEM: 31986 MB 31989 MB.
[2026-09-06 23:36:36.422] INFO: CPU: 1.4 % 6.2 %, MEM: 31987 MB 31992 MB.
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

[2026-09-06 23:36:36.523] INFO: Average CPU usage in system: 6.1 %.
[2026-09-06 23:36:36.523] INFO: Average MEM usage in system: 31992 MB.
[2026-09-06 23:36:36.523] INFO: Max MEM usage in system: 32000 MB.
[2026-09-06 23:36:36.523] INFO: Average CPU used by DEE process: 0 %.
[2026-09-06 23:36:36.523] INFO: Max MEM used by DEE process: 35 MB.
Time elapsed: 2.7757 seconds
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


Execution time: 0.000405 seconds
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


Execution time: 0.0007712 seconds
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



Execution time: 0.000612 seconds
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


Execution time: 0.0006291 seconds
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


Execution time: 0.0007168 seconds
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



Execution time: 0.0005964 seconds
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



Execution time: 0.000589 seconds
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


Execution time: 0.0004116 seconds
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
                                           1               Kulakl�klar (AirPods Pro) (2 channels)
                                           2               Realtek Digital Output (Realtek(R) Audio) (2 channels)
                                           3               VG27AQML1A (NVIDIA High Definition Audio) (2 channels)
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

## `cmdline_atmos_conversion_tool.exe --help` (Dolby Atmos Conversion Tool 2.1.2)

```text
Dolby Atmos Conversion Tool. A Dolby Atmos media file format and frames-per-second conversion tool.

Option list:
  -h [ --help ]                       Displays help (full list of Dolby Atmos Conversion Tool 
                                      command-line options).
  -v [ --version ]                    Displays the application version and exits the application.
  -V [ --verbose ]                    Enables verbose information for debugging.
  -i [ --pm_in ] arg                  Specifies the path to the input Dolby Atmos master file.
  -o [ --output_path ] arg (=.)       Specifies the output path for the audio and metadata files. 
                                      The default is the current working directory.
  -f [ --output_format ] arg (=atmos) Specifies the output file format option: atmos, rpl, wav, or 
                                      mxf.
  --source_fps arg                    Specifies the frame rate of the source master file in frames 
                                      per second (FPS). This option will set the frame rate of the 
                                      source master file that does not specify one. Valid values 
                                      are dependent on the frame rate supported by the source 
                                      master. They can include: 23.976, 24, 25, 29.97, 29.97df, 30.
  --target_fps arg                    Specifies the frame rate of the target master file. Valid 
                                      values are dependent on the frame rate supported by the 
                                      target master. They can include: 23.976, 24, 25, 29.97, 
                                      29.97df, 30. Frame rates of 29.97 and 29.97df are not 
                                      supported for mxf output format. Frame rates supported by rpl
                                      format are 24, 25 and 30 FPS. Numbers near 23.976 and 29.97 
                                      will be rounded to 24*1000/1001 and 30*1000/1001. If this 
                                      value is equal to frame rate in the source file, frame rate 
                                      conversion is bypassed and the tool does a format conversion 
                                      only.
  --no_ffoa                           Don't include FFOA when creating the new master.
  --ffoa arg                          Specifies first frame of action (FFOA).  If none is 
                                      specified, the source master FFOA will be used.
  -l [ --primary_lang ] arg           Specifies IMF IAB soundfield primary spoken language as RFC 
                                      5646 code value. Applicable only for mxf output format. If 
                                      none is specified for an input mxf master, the primary 
                                      language of the master is passed through to the output. If 
                                      none is specified for all other input master types, "en" will
                                      be used.
  --list_languages                    Displays full list of supported RFC 5646 primary spoken 
                                      language codes.
  --quality arg (=0)                  Specifies the quality of the libsamplerate conversion during 
                                      a frame rate or sample rate conversion, where 0=best_quality 
                                      and 4=linear.
  --disable_multithreading            Disables multithreading and uses a single thread for the 
                                      resampling during a frame rate or sample rate conversion.
  --bypass_resampling                 Disables resampling of the audio content during a frame rate 
                                      conversion (resampling only happens for sample rate 
                                      conversions if needed).
  --target_sample_rate arg (=48000)   Specifies the sample rate in Hz of the target master, valid 
                                      options are: 48000 or 96000.
                                      Valid sample rate conversions:
                                      - 48000 All formats     -> 48000 All formats
                                      - 96000 .atmos, ADM BWF -> 48000 .atmos, ADM BWF, IMF IAB
                                      - 96000 .atmos, ADM BWF -> 96000 ADM BWF
  --set_warp_mode arg                 Specifies the warp mode for the target master, valid options 
                                      are:
                                      - downmix_loro, Standard (Lo/Ro)
                                      - downmix_pliix, Dolby Pro Logix IIx
                                      - warping, Direct render with room balance
                                      - normal, Direct render
  --trim_start arg (=0)               Trims n seconds from the beginning of the input master. 
                                      Applied before prepending or appending silence.
  --trim_duration arg (=0)            Trims input master to n seconds after the defined start. 
                                      Applied before prepending or appending silence.
  --prepend_silence arg (=0)          Prepends n seconds of audio silence to the beginning of the 
                                      input master.
  --append_silence arg (=0)           Appends n seconds of audio silence to the end of the input 
                                      master.

```
